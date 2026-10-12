//! The `sweepgen` protocol and the sweep work packet (Go: `runtime/mgcsweep.go`).
//!
//! Relative to the space's sweep generation `h`, a span's `sweepgen` means
//! (Go, `runtime/mheap.go:494-500`):
//!
//! | span `sweepgen` | state |
//! |---|---|
//! | `h - 2` | needs sweeping |
//! | `h - 1` | being swept |
//! | `h` | swept and ready to use |
//! | `h + 1` | cached by an allocator before sweeping began; still cached and needs sweeping |
//! | `h + 3` | swept and then cached; still cached |
//!
//! `h` grows by 2 at the end of each mark phase ([`super::SpanSpace::release_after_mark`]), which
//! turns every swept span (`h`) into one that needs sweeping (`h - 2`) and every cached span
//! (`h + 3`) into a stale cached span (`h + 1`). Sweepers take ownership of a span with a CAS from
//! `h - 2` to `h - 1` ([`try_acquire`]), so allocator sweeping, sweep packets and `sweep_one` can
//! all run at the same time.

use super::SpanSpace;
use crate::scheduler::{GCWork, GCWorker};
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::atomic::{AtomicU32, Ordering};

/// What a span's `sweepgen` means relative to the space's `sweepgen` (see the module docs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SweepState {
    NeedsSweep,
    BeingSwept,
    Swept,
    CachedStale,
    Cached,
}

/// Classify a span's `sweepgen` against the space's `h`.
pub fn sweep_state(span_sweepgen: u32, h: u32) -> SweepState {
    match span_sweepgen.wrapping_sub(h) {
        x if x == 2u32.wrapping_neg() => SweepState::NeedsSweep,
        x if x == 1u32.wrapping_neg() => SweepState::BeingSwept,
        0 => SweepState::Swept,
        1 => SweepState::CachedStale,
        3 => SweepState::Cached,
        _ => panic!("bad span sweepgen {span_sweepgen} (heap sweepgen {h})"),
    }
}

/// Try to take sweeping ownership of a span (Go: `sweepLocker.tryAcquire`,
/// `runtime/mgcsweep.go:342`): CAS its `sweepgen` from `h - 2` to `h - 1`.
pub fn try_acquire(span_sweepgen: &AtomicU32, h: u32) -> bool {
    // Check before attempting to CAS.
    if span_sweepgen.load(Ordering::Acquire) != h.wrapping_sub(2) {
        return false;
    }
    span_sweepgen
        .compare_exchange(
            h.wrapping_sub(2),
            h.wrapping_sub(1),
            Ordering::AcqRel,
            Ordering::Relaxed,
        )
        .is_ok()
}

/// A sweep work packet (Go's background sweeper, `bgsweep`). It sweeps unswept spans from the
/// central sets until there are none left. Several packets can run at once; each starts at a
/// different span class to reduce contention.
///
/// The packets can run in a stop-the-world `Release` stage (eager sweeping), in the `Concurrent`
/// bucket after a pause (background sweeping), or in `Prepare` to finish the previous cycle's
/// sweeping. See [`SpanSpace::schedule_sweep`].
pub struct SweepSpans<VM: VMBinding> {
    pub(super) space: &'static SpanSpace<VM>,
    pub(super) first_class: usize,
}

impl<VM: VMBinding> GCWork<VM> for SweepSpans<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let mut pages = 0;
        loop {
            let swept = self.space.sweep_one_from(self.first_class);
            if swept == 0 {
                break;
            }
            pages += swept;
        }
        trace!("SweepSpans: swept {} pages", pages);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go's sweepgen state machine over a few cycles, including wrap-around of the counter.
    #[test]
    fn sweepgen_state_machine() {
        for start in [0u32, 4, u32::MAX - 3] {
            let mut h = start;
            let s = AtomicU32::new(h); // a fresh span is swept
            let cached = AtomicU32::new(h.wrapping_add(3)); // and one cached by an allocator
            assert_eq!(sweep_state(s.load(Ordering::Relaxed), h), SweepState::Swept);
            assert_eq!(
                sweep_state(cached.load(Ordering::Relaxed), h),
                SweepState::Cached
            );
            assert!(!try_acquire(&s, h), "a swept span cannot be acquired");

            // Mark termination: h += 2.
            h = h.wrapping_add(2);
            assert_eq!(
                sweep_state(s.load(Ordering::Relaxed), h),
                SweepState::NeedsSweep
            );
            assert_eq!(
                sweep_state(cached.load(Ordering::Relaxed), h),
                SweepState::CachedStale
            );
            assert!(
                !try_acquire(&cached, h),
                "cached spans are swept by their owner"
            );

            // Exactly one of two sweepers wins.
            assert!(try_acquire(&s, h));
            assert!(!try_acquire(&s, h));
            assert_eq!(
                sweep_state(s.load(Ordering::Relaxed), h),
                SweepState::BeingSwept
            );
            s.store(h, Ordering::Release); // sweep done
            assert_eq!(sweep_state(s.load(Ordering::Relaxed), h), SweepState::Swept);

            // The owner uncaches a stale span: h - 1, sweep, h (Go's `uncacheSpan`).
            cached.store(h.wrapping_sub(1), Ordering::Release);
            assert_eq!(
                sweep_state(cached.load(Ordering::Relaxed), h),
                SweepState::BeingSwept
            );
            cached.store(h, Ordering::Release);
            // Caching it again: h + 3.
            cached.store(h.wrapping_add(3), Ordering::Release);
            assert_eq!(
                sweep_state(cached.load(Ordering::Relaxed), h),
                SweepState::Cached
            );
        }
    }

    #[test]
    #[should_panic]
    fn sweepgen_out_of_range() {
        sweep_state(10, 4);
    }
}
