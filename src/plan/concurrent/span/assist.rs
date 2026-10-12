//! Mark assists (design §5.11, Stage A).
//!
//! Go makes a goroutine that allocates during marking pay for it with mark work
//! (`gcAssistAlloc`, `runtime/mgcmark.go:499-824`).  Here a mutator in debt first steals
//! background credit, as in Go.  If that is not enough, it does not run mark work itself:
//! it parks, and the plan activates one more GC worker while it is parked.  Workers pay the
//! parked mutators' debts in FIFO order as they finish scan work (Go's `gcFlushBgCredit`).

use super::pacer::{FeedbackPacer, GC_OVER_ASSIST_WORK};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A parked mutator's outstanding scan work.
pub(super) struct AssistRequest {
    remaining_work: AtomicI64,
    done: AtomicBool,
}

impl AssistRequest {
    pub(super) fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
}

/// The assist queue (Go: `work.assistQueue`).
#[derive(Default)]
pub(super) struct Assists {
    queue: Mutex<VecDeque<Arc<AssistRequest>>>,
    /// The number of parked mutators.
    parked: AtomicUsize,
    /// Statistics for the current cycle.
    pub(super) count: AtomicUsize,
    pub(super) parked_count: AtomicUsize,
}

/// What an assisting mutator has to do.
pub(super) enum AssistOutcome {
    /// The debt is paid.  Add this many bytes to the mutator's credit.
    Paid(isize),
    /// Park until the request is done (or marking ends), then add the bytes to the credit.
    Park(Arc<AssistRequest>, isize),
}

impl Assists {
    /// Start an assist for a mutator with `credit` (negative) bytes of allocation credit.
    // Go: runtime/mgcmark.go:425-498 (gcAssistAlloc, 2c3c1caad2), without the CPU limiter
    // and without running mark work on the mutator.
    pub(super) fn start(&self, pacer: &FeedbackPacer, credit: isize) -> AssistOutcome {
        debug_assert!(credit < 0);
        self.count.fetch_add(1, Ordering::Relaxed);
        let assist_work_per_byte = pacer.assist_work_per_byte();
        let assist_bytes_per_work = pacer.assist_bytes_per_work();
        let mut debt_bytes = -(credit as i64);
        let mut scan_work = (assist_work_per_byte * debt_bytes as f64) as i64;
        if scan_work < GC_OVER_ASSIST_WORK {
            scan_work = GC_OVER_ASSIST_WORK;
            debt_bytes = (assist_bytes_per_work * scan_work as f64) as i64;
        }

        // Steal as much credit as we can from the background GC's scan credit.
        let bg_credit = pacer.bg_scan_credit().load(Ordering::Relaxed);
        if bg_credit > 0 {
            let stolen = bg_credit.min(scan_work);
            pacer.bg_scan_credit().fetch_sub(stolen, Ordering::Relaxed);
            scan_work -= stolen;
            if scan_work == 0 {
                // We were able to steal all of the credit we needed.
                return AssistOutcome::Paid(debt_bytes as isize);
            }
        }

        // Queue the rest.  The debt is paid in full when the request is done.
        let request = Arc::new(AssistRequest {
            remaining_work: AtomicI64::new(scan_work),
            done: AtomicBool::new(false),
        });
        self.queue.lock().unwrap().push_back(request.clone());
        self.parked.fetch_add(1, Ordering::SeqCst);
        self.parked_count.fetch_add(1, Ordering::Relaxed);
        AssistOutcome::Park(request, debt_bytes as isize)
    }

    /// A parked mutator has woken up.
    pub(super) fn unparked(&self) {
        // `release_all` may have reset the count already.
        let _ = self
            .parked
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    }

    /// Workers did `scan_work` units of scan work.  Pay the queued assists first, and add the
    /// rest to the background credit.  Returns `true` if a request was completed.
    // Go: runtime/mgcmark.go:756-824 (gcFlushBgCredit, 2c3c1caad2)
    pub(super) fn flush_bg_credit(&self, pacer: &FeedbackPacer, mut scan_work: i64) -> bool {
        if self.parked.load(Ordering::Relaxed) == 0 {
            // Fast path; there were no blocked assists.
            pacer
                .bg_scan_credit()
                .fetch_add(scan_work, Ordering::Relaxed);
            return false;
        }
        let mut woke = false;
        {
            let mut queue = self.queue.lock().unwrap();
            while scan_work > 0 {
                let Some(front) = queue.front() else {
                    break;
                };
                let need = front.remaining_work.load(Ordering::Relaxed);
                if need <= scan_work {
                    // Satisfy this entire assist debt.
                    scan_work -= need;
                    front.done.store(true, Ordering::SeqCst);
                    queue.pop_front();
                    woke = true;
                } else {
                    // Partially satisfy this assist.
                    front.remaining_work.fetch_sub(scan_work, Ordering::Relaxed);
                    scan_work = 0;
                }
            }
        }
        if scan_work > 0 {
            pacer
                .bg_scan_credit()
                .fetch_add(scan_work, Ordering::Relaxed);
        }
        woke
    }

    /// Forgive every queued assist and wake them, when marking is done or has run out of work
    /// (Go: `gcWakeAllAssists`).  Returns `true` if any was queued.
    pub(super) fn release_all(&self) -> bool {
        let mut queue = self.queue.lock().unwrap();
        let any = !queue.is_empty();
        for request in queue.drain(..) {
            request.done.store(true, Ordering::SeqCst);
        }
        self.parked.store(0, Ordering::SeqCst);
        any
    }

    /// Reset the statistics at the start of a cycle.
    pub(super) fn reset_stats(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.parked_count.store(0, Ordering::Relaxed);
    }
}
