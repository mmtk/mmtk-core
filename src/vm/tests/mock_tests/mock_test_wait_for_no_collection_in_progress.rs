// GITHUB-CI: MMTK_PLAN=Immix,ConcurrentImmix

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::util::{OpaquePointer, VMMutatorThread, VMThread};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Sync {
    gc_started: bool,
    thread_b_is_retrying: bool,
    release_gc: bool,
    thread_a_finished: bool,
    thread_b_finished: bool,
}

#[derive(Default)]
struct SharedData {
    mutex: Mutex<Sync>,
    gc_started_cond: Condvar,
    thread_b_is_retrying_cond: Condvar,
    release_gc_cond: Condvar,
    thread_a_finished_cond: Condvar,
    thread_b_finished_cond: Condvar,
}

lazy_static! {
    static ref SHARED_DATA: SharedData = Default::default();
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
                    let mut sync = SHARED_DATA.mutex.lock().unwrap();
                    sync.gc_started = true;
                    SHARED_DATA.gc_started_cond.notify_all();
                    while !sync.release_gc {
                        sync = SHARED_DATA.release_gc_cond.wait(sync).unwrap();
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
                {
                    let mut sync = SHARED_DATA.mutex.lock().unwrap();
                    sync.thread_a_finished = true;
                    SHARED_DATA.thread_a_finished_cond.notify_all();
                }
            });

            // Wait until the GC has actually started (i.e. is "in progress").
            {
                let mut sync = SHARED_DATA.mutex.lock().unwrap();
                while !sync.gc_started {
                    sync = SHARED_DATA.gc_started_cond.wait(sync).unwrap();
                }
            }

            // `disable_collection()` must fail while a GC is in progress, rather than racing
            // past it.
            assert_eq!(
                mmtk.disable_collection(),
                Err(GcStatus::PauseRequested),
                "disable_collection() succeeded while a GC was still in progress"
            );

            // Thread B: as `disable_collection()`'s documentation instructs, retry after a GC until
            // it succeeds.
            let thread_to_disable_gc = std::thread::spawn(move || {
                while mmtk.disable_collection().is_err() {
                    let mut sync = SHARED_DATA.mutex.lock().unwrap();
                    sync.thread_b_is_retrying = true;
                    SHARED_DATA.thread_b_is_retrying_cond.notify_all();
                    while !sync.gc_started {
                        sync = SHARED_DATA.gc_started_cond.wait(sync).unwrap();
                    }
                }

                {
                    let mut sync = SHARED_DATA.mutex.lock().unwrap();
                    sync.thread_b_finished = true;
                    SHARED_DATA.thread_b_finished_cond.notify_all();
                }
            });

            // Wait until Thread B starts retrying.
            {
                let mut sync = SHARED_DATA.mutex.lock().unwrap();
                while !sync.thread_b_is_retrying {
                    sync = SHARED_DATA.thread_b_is_retrying_cond.wait(sync).unwrap();
                }
            }
            assert!(
                !thread_to_disable_gc.is_finished(),
                "disable_collection() succeeded while a GC was still in progress"
            );

            {
                let mut sync = SHARED_DATA.mutex.lock().unwrap();

                // Let thread A's GC finish.
                sync.release_gc = true;
                SHARED_DATA.release_gc_cond.notify_all();

                // Wait until thread A finishes.
                while !sync.thread_a_finished {
                    let (new_sync, timeout_result) = SHARED_DATA
                        .thread_a_finished_cond
                        .wait_timeout(sync, TIMEOUT)
                        .unwrap();
                    assert!(
                        !timeout_result.timed_out(),
                        "the GC-triggering thread did not finish in time"
                    );
                    sync = new_sync;
                }
            }

            // This test does not spawn real GC worker threads, so nothing else transitions the
            // GC status away from `PauseRequested` (which is what thread A's request set it to).
            // Normally the scheduler does this via `notify_mutators_paused()` (-> `InPause`) once
            // all mutators have stopped, then `on_gc_finished()` (-> `NotInGC`) once the
            // collection completes; simulate that sequence here.
            mmtk.state.gc_status.set_in_pause();
            mmtk.state.gc_status.set_not_in_gc();

            // Now that the GC has finished, thread B's retry should succeed and it should finish.
            {
                let mut sync = SHARED_DATA.mutex.lock().unwrap();

                // Wait until thread B finishes.
                while !sync.thread_b_finished {
                    let (new_sync, timeout_result) = SHARED_DATA
                        .thread_b_finished_cond
                        .wait_timeout(sync, TIMEOUT)
                        .unwrap();
                    assert!(
                        !timeout_result.timed_out(),
                        "disable_collection() did not succeed after the GC finished",
                    );
                    sync = new_sync;
                }
            }

            assert!(mmtk.enable_collection());
            thread_to_trigger_gc.join().unwrap();
            thread_to_disable_gc.join().unwrap();
        },
        no_cleanup,
    )
}
