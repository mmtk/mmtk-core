use std::{
    sync::{Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::Duration,
};

use super::mock_test_prelude::*;
use crate::{
    util::{options::GCTriggerSelector, Address, OpaquePointer, VMThread, VMWorkerThread},
    MMTKBuilder, MMTK,
};

#[derive(Default)]
struct ForkTestShared {
    sync: Mutex<ForkTestSync>,
    all_threads_spawn: Condvar,
    all_threads_exited: Condvar,
    all_threads_running: Condvar,
}

#[derive(Default)]
struct ForkTestSync {
    join_handles: Vec<JoinHandle<()>>,
    /// Number of threads spawn.
    spawn_threds: usize,
    /// Number of threads that have actually entered our entry-point function.
    running_threads: usize,
    /// Number of threads that have returned from `memory_manager::start_worker`.
    exited_threads: usize,
}

lazy_static! {
    static ref SHARED: ForkTestShared = ForkTestShared::default();
}

// We fix the number of threads so that we can assert the number of GC threads spawn.
const NUM_WORKER_THREADS: usize = 4;

// Don't block the CI.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A convenient wrapper that panics on timeout.
fn wait_timeout_while<'a, T, F>(
    guard: MutexGuard<'a, T>,
    condvar: &Condvar,
    condition: F,
) -> MutexGuard<'a, T>
where
    F: FnMut(&mut T) -> bool,
{
    let (guard, timeout_result) = condvar
        .wait_timeout_while(guard, TIMEOUT, condition)
        .unwrap();
    assert!(!timeout_result.timed_out());
    guard
}

fn simple_spawn_gc_thread(
    _vm_thread: VMThread,
    context: GCThreadContext<MockVM>,
    mmtk: &'static MMTK<MockVM>,
) {
    let GCThreadContext::Worker(worker) = context;
    let join_handle = std::thread::spawn(move || {
        let ordinal = worker.ordinal;
        println!("GC thread starting. Ordinal: {ordinal}");

        {
            let mut sync = SHARED.sync.lock().unwrap();
            sync.running_threads += 1;
            if sync.running_threads == NUM_WORKER_THREADS {
                SHARED.all_threads_running.notify_all();
            }
        }

        let gc_thread_tls = VMWorkerThread(VMThread(OpaquePointer::from_address(Address::ZERO)));
        memory_manager::start_worker(mmtk, gc_thread_tls, worker);

        {
            let mut sync = SHARED.sync.lock().unwrap();
            sync.running_threads -= 1;
            sync.exited_threads += 1;
            if sync.exited_threads == NUM_WORKER_THREADS {
                SHARED.all_threads_exited.notify_all();
            }
        }

        println!("GC thread stopped. Ordinal: {ordinal}");
    });

    {
        let mut sync = SHARED.sync.lock().unwrap();
        sync.join_handles.push(join_handle);
        sync.spawn_threds += 1;
        if sync.spawn_threds == NUM_WORKER_THREADS {
            SHARED.all_threads_spawn.notify_all();
        }
    }
}

/// Test the `initialize_collection` function with actual running GC threads, and the functions for
/// supporting forking.
#[test]
pub fn test_initialize_collection_and_fork() {
    // We don't use fixtures or `with_mockvm` because we want to precisely control the
    // initialization process.
    let mut builder = MMTKBuilder::new();
    // The exact heap size doesn't matter because we don't even allocate.
    let trigger = GCTriggerSelector::FixedHeapSize(1024 * 1024);
    builder.options.gc_trigger.set(trigger);
    builder.options.threads.set(NUM_WORKER_THREADS);
    let mmtk: &'static mut MMTK<MockVM> = Box::leak(Box::new(builder.build::<MockVM>()));

    let mock_vm = MockVM {
        spawn_gc_thread: MockMethod::new_fixed(Box::new(|(vm_thread, context)| {
            simple_spawn_gc_thread(vm_thread, context, mmtk)
        })),
        ..Default::default()
    };
    write_mockvm(move |mock_vm_ref| *mock_vm_ref = mock_vm);

    let test_thread_tls = VMThread(OpaquePointer::from_address(Address::ZERO));

    // Initialize collection.  This will spawn GC worker threads.
    mmtk.initialize_collection(test_thread_tls);

    // Wait for GC workers to be spawned, and get their join handles.
    let join_handles = {
        println!("Waiting for GC worker threads to be spawn");
        let sync = SHARED.sync.lock().unwrap();

        // We wait for `all_threads_spawn` instead of `all_threads_running`.  It is not necessary
        // to wait for all GC worker threads to enter `memory_manager::start_worker` before calling
        // `prepare_to_fork`.  The impementation of `initialize_collection` and `prepare_to_fork`
        // should be robust against GC worker threads that start slower than usual.
        let mut sync = wait_timeout_while(sync, &SHARED.all_threads_spawn, |sync| {
            sync.spawn_threds < NUM_WORKER_THREADS
        });

        // Take join handles out of `SHARED.sync` so that the main thread can join them without
        // holding the Mutex, and GC workers can acquire the mutex and mutate `SHARED.sync`.
        std::mem::take(&mut sync.join_handles)
    };

    assert_eq!(join_handles.len(), NUM_WORKER_THREADS);

    // Now we prepare to fork.  GC worker threads should go down.
    mmtk.prepare_to_fork();

    println!("Waiting for GC worker threads to stop");

    {
        // In theory, we can just join the join handles, and is unnecessary to wait for
        // `SHARED.all_threads_exited`.  This is a workaround for the fact that
        // `JoinHandle::join()` does not have a variant that supports timeout.  We use
        // `wait_timeout_while` so that it won't hang the CI.  When the condvar
        // `all_threads_exited` is notified, all GC workers will have returned from
        // `memory_manager::start_worker`, and it is unlikely that the `join_handle.join()` below
        // will block for too long.
        let sync = SHARED.sync.lock().unwrap();
        let _sync = wait_timeout_while(sync, &SHARED.all_threads_exited, |sync| {
            sync.exited_threads < NUM_WORKER_THREADS
        });
    }

    for join_handle in join_handles {
        // TODO: PThread has `pthread_timedjoin_np`, but the `JoinHandle` in the Rust standard
        // library doesn't have a variant that supports timeout.  Let's wait for the Rust library
        // to update.
        join_handle.join().unwrap();
    }

    println!("All GC worker threads stopped");

    {
        let mut sync = SHARED.sync.lock().unwrap();
        assert_eq!(sync.running_threads, 0);

        // Reset counters.
        sync.spawn_threds = 0;
        sync.exited_threads = 0;
    }

    // We don't actually call `fork()` in this test, but we pretend we have called `fork()`.
    // We now try to resume GC workers.
    mmtk.after_fork(test_thread_tls);

    {
        println!("Waiting for GC worker threads to be running after calling `after_fork`");
        let sync = SHARED.sync.lock().unwrap();
        let _sync = wait_timeout_while(sync, &SHARED.all_threads_running, |sync| {
            sync.running_threads < NUM_WORKER_THREADS
        });
    }

    println!("All GC worker threads are up and running.");
}
