// GITHUB-CI: MMTK_PLAN=Immix,ConcurrentImmix

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
/// `disable_collection()` on another thread must fail (return `false`) rather than racing past
/// it, and must only succeed once that GC has actually finished.
#[test]
pub fn disable_collection_fails_while_gc_in_progress() {
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
            let thread_to_trigger_gc = std::thread::spawn(move || {
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

            // `disable_collection()` must fail while a GC is in progress, rather than racing
            // past it.
            assert!(
                !mmtk.disable_collection(),
                "disable_collection() succeeded while a GC was still in progress"
            );

            // Thread B: as `disable_collection()`'s documentation instructs, retry (yielding
            // between attempts) until it succeeds.
            let thread_to_disable_gc = std::thread::spawn(move || {
                while !mmtk.disable_collection() {
                    std::thread::yield_now();
                }
            });

            // Give thread B a chance to run, then confirm it is still retrying.
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !thread_to_disable_gc.is_finished(),
                "disable_collection() succeeded while a GC was still in progress"
            );

            // Let thread A's GC finish.
            {
                let (lock, cvar) = &*SYNC;
                let mut sync = lock.lock().unwrap();
                sync.release_gc = true;
                cvar.notify_all();
            }
            wait_until(
                || thread_to_trigger_gc.is_finished(),
                "the GC-triggering thread did not finish in time",
            );
            // This test does not spawn real GC worker threads, so nothing else transitions the
            // GC status away from `PauseRequested` (which is what thread A's request set it to).
            // Normally the scheduler does this via `notify_mutators_paused()` (-> `InPause`) once
            // all mutators have stopped, then `on_gc_finished()` (-> `NotInGC`) once the
            // collection completes; simulate that sequence here.
            mmtk.state.gc_status.set_in_pause();
            mmtk.state.gc_status.set_not_in_gc();

            // Now that the GC has finished, thread B's retry should succeed and it should finish.
            wait_until(
                || thread_to_disable_gc.is_finished(),
                "disable_collection() did not succeed after the GC finished",
            );

            mmtk.enable_collection();
            thread_to_trigger_gc.join().unwrap();
            thread_to_disable_gc.join().unwrap();
        },
        no_cleanup,
    )
}
