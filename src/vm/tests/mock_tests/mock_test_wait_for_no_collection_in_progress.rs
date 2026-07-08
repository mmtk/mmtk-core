use super::mock_test_prelude::*;
use crate::util::{OpaquePointer, VMMutatorThread, VMThread};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Sync {
    gc_started: bool,
    release_gc: bool,
}

lazy_static! {
    static ref SYNC: (Mutex<Sync>, Condvar) = (Mutex::new(Sync::default()), Condvar::new());
}

/// Poll `condition` until it is true, or panic if `TIMEOUT` elapses first. Used instead of an
/// unbounded `JoinHandle::join()` so a regression turns into a clear test failure rather than a
/// hung test process.
fn wait_until(mut condition: impl FnMut() -> bool, msg: &str) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < TIMEOUT, "{}", msg);
        std::thread::yield_now();
    }
}

/// Regression test for issue mmtk/mmtk-julia#278: if a GC is already in progress on one thread,
/// another thread that calls `disable_collection()` followed by
/// `wait_for_no_collection_in_progress()` must block until that GC actually finishes, rather
/// than racing past it.
#[test]
pub fn wait_for_no_collection_in_progress_blocks_until_gc_finishes() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                // Simulate a GC that is "in progress": signal that we have started, then block
                // until the test tells us to finish.
                block_for_gc: MockMethod::new_fixed(Box::new(|_| {
                    let (lock, cvar) = &*SYNC;
                    let mut sync = lock.lock().unwrap();
                    sync.gc_started = true;
                    cvar.notify_all();
                    while !sync.release_gc {
                        sync = cvar.wait(sync).unwrap();
                    }
                })),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_heapsize(1024 * 1024);
            let mmtk = fixture.mmtk();

            // Thread A: trigger a GC. `handle_user_collection_request` blocks the calling
            // thread in (our mocked) `block_for_gc` until the GC finishes.
            let gc_thread = std::thread::spawn(move || {
                let tls = VMMutatorThread(VMThread(OpaquePointer::UNINITIALIZED));
                memory_manager::handle_user_collection_request(mmtk, tls);
            });

            // Wait until the GC has actually started (i.e. is "in progress").
            {
                let (lock, cvar) = &*SYNC;
                let mut sync = lock.lock().unwrap();
                while !sync.gc_started {
                    sync = cvar.wait(sync).unwrap();
                }
            }

            // Thread B: disable collection and wait for no collection to be in progress. This
            // must block, because the GC triggered by thread A is still running.
            let waiter = std::thread::spawn(move || {
                mmtk.disable_collection();
                mmtk.wait_for_no_collection_in_progress();
            });

            // Give thread B a chance to run, then confirm it is still blocked.
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !waiter.is_finished(),
                "wait_for_no_collection_in_progress() returned while a GC was still in progress"
            );

            // Let thread A's GC finish.
            {
                let (lock, cvar) = &*SYNC;
                let mut sync = lock.lock().unwrap();
                sync.release_gc = true;
                cvar.notify_all();
            }
            wait_until(
                || gc_thread.is_finished(),
                "the GC-triggering thread did not finish in time",
            );
            // This test does not spawn real GC worker threads, so nothing else clears the GC
            // request that thread A made. Normally the scheduler does this once all mutators
            // have stopped and the collection completes; simulate that here.
            mmtk.gc_trigger.clear_request();

            // Now that the GC has finished, thread B should unblock.
            wait_until(
                || waiter.is_finished(),
                "wait_for_no_collection_in_progress() did not return after the GC finished",
            );

            mmtk.enable_collection();
            gc_thread.join().unwrap();
            waiter.join().unwrap();
        },
        no_cleanup,
    )
}
