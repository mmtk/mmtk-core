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
struct ShutdownTestShared {
    sync: Mutex<ShutdownTestSync>,
    all_threads_spawned: Condvar,
    all_threads_exited: Condvar,
}

#[derive(Default)]
struct ShutdownTestSync {
    join_handles: Vec<JoinHandle<()>>,
    spawned_threads: usize,
    exited_threads: usize,
}

lazy_static! {
    static ref SHARED: ShutdownTestShared = ShutdownTestShared::default();
}

const NUM_WORKER_THREADS: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(5);

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

        let gc_thread_tls = VMWorkerThread(VMThread(OpaquePointer::from_address(Address::ZERO)));
        memory_manager::start_worker(mmtk, gc_thread_tls, worker);

        let mut sync = SHARED.sync.lock().unwrap();
        sync.exited_threads += 1;
        if sync.exited_threads == NUM_WORKER_THREADS {
            SHARED.all_threads_exited.notify_all();
        }

        println!("GC thread stopped. Ordinal: {ordinal}");
    });

    let mut sync = SHARED.sync.lock().unwrap();
    sync.join_handles.push(join_handle);
    sync.spawned_threads += 1;
    if sync.spawned_threads == NUM_WORKER_THREADS {
        SHARED.all_threads_spawned.notify_all();
    }
}

#[test]
pub fn test_shutdown_stops_gc_threads() {
    let mut builder = MMTKBuilder::new();
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
    mmtk.initialize_collection(test_thread_tls);

    let join_handles = {
        let sync = SHARED.sync.lock().unwrap();
        let mut sync = wait_timeout_while(sync, &SHARED.all_threads_spawned, |sync| {
            sync.spawned_threads < NUM_WORKER_THREADS
        });
        std::mem::take(&mut sync.join_handles)
    };

    assert_eq!(join_handles.len(), NUM_WORKER_THREADS);

    memory_manager::mmtk_shutdown(mmtk);

    println!("Waiting for GC worker threads to stop");

    {
        let sync = SHARED.sync.lock().unwrap();
        let sync = wait_timeout_while(sync, &SHARED.all_threads_exited, |sync| {
            sync.exited_threads < NUM_WORKER_THREADS
        });
        assert_eq!(sync.exited_threads, NUM_WORKER_THREADS);
    }

    assert!(!mmtk.state.is_initialized());

    for join_handle in join_handles {
        join_handle.join().unwrap();
    }
}
