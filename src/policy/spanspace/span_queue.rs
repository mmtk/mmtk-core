//! Green Tea span scanning: the per-span state (Go: `spanInlineMarkBits`,
//! `runtime/mgcmark_greenteagc.go` (2c3c1caad2)). Design §3.4 and §5.12.
//!
//! Spans of [`SpanClass::uses_span_queue`] classes (one-page spans of 16-512 B scan objects) are
//! scanned span-at-a-time while marking concurrently:
//!
//! - Marking an object sets its mark bit and then tries to take ownership of its span
//!   ([`Span::try_acquire`]). Only the marker that takes ownership queues the span, as a
//!   [`SpanWork`]; other markers leave their object to the owner.
//! - Scanning a span ([`Span::release_ownership`] then [`Span::take_unscanned`]) releases the
//!   ownership **first**, and then scans every object that is marked but not yet scanned
//!   (`marks &^ scans`), setting their scan bits. A marker that finds the span owned has set its
//!   mark bit before it looked at `owned`, so the owner's later pass sees it; a marker that comes
//!   after the release takes ownership again and queues the span again.
//! - Objects allocated black get their scan bit (before their mark bit), so they are never
//!   scanned. Objects that are marked and queued one by one (the generic `trace_object`) also get
//!   their scan bit, so a span scan does not scan them again.
//!
//! Go keeps `scans`, `owned` and `marks` in a 128-byte block at the end of the span page. Here the
//! mark bits stay in the span descriptor ([`Span::mark_bits`], as for every other span), and the
//! scan bits and `owned` are next to them in the descriptor (deviation GT-1).
//!
//! All operations on `owned`, and the mark and scan bit operations of the Green Tea paths, use
//! `SeqCst`: a marker that finds the span owned with a plain load (`owned == MANY`) must have its
//! mark bit seen by the owner, whose ownership release happens later in the total order. (Go's
//! `atomic.Or8`/`Load8`/`Xchg8` are sequentially consistent too.)

use super::classes::*;
use super::span::{Span, SpanId};
use std::sync::atomic::Ordering::*;

/// Go: `spanScanOwnership`.
pub(crate) const SPAN_SCAN_UNOWNED: u8 = 0;
/// Only one object was marked relative to the scan bits since the span was queued (the one in the
/// [`SpanWork`]).
pub(crate) const SPAN_SCAN_ONE_MARK: u8 = 1;
/// More than one object may have been marked since the span was queued.
pub(crate) const SPAN_SCAN_MANY_MARK: u8 = 2;

/// A queued span (Go: `objptr`, a span base with the index of the object whose marking queued
/// it).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SpanWork {
    pub(crate) id: SpanId,
    pub(crate) index: u32,
}

impl Span {
    /// Go: `spanInlineMarkBits.tryAcquire`. Try to take ownership of the span for scanning. On
    /// success, the caller must queue the span.
    #[inline(always)]
    pub(crate) fn try_acquire(&self) -> bool {
        let owned = self.owned.load(SeqCst);
        if owned == SPAN_SCAN_UNOWNED {
            // Try to mark the span as having only one object marked.
            if self.owned.fetch_or(SPAN_SCAN_ONE_MARK, SeqCst) == SPAN_SCAN_UNOWNED {
                return true;
            }
            // We raced with someone else and saw ONE_MARK or more: fall through.
        } else if owned != SPAN_SCAN_ONE_MARK {
            return false;
        }
        // We may be the first to set *any* bit on `owned`. In that case, we still need to make
        // sure the span is queued.
        self.owned.fetch_or(SPAN_SCAN_MANY_MARK, SeqCst) == SPAN_SCAN_UNOWNED
    }

    /// Go: `spanInlineMarkBits.release`. Release the span, so that the next marker queues it
    /// again. Returns an upper bound on how many objects were marked since the span was queued
    /// (`ONE_MARK` or `MANY_MARK`).
    #[inline(always)]
    pub(crate) fn release_ownership(&self) -> u8 {
        self.owned.swap(SPAN_SCAN_UNOWNED, SeqCst)
    }

    /// Whether a span is owned for scanning (queued or being scanned).
    pub fn is_owned(&self) -> bool {
        self.owned.load(SeqCst) != SPAN_SCAN_UNOWNED
    }

    /// Atomically set the mark bit of object `index`, for the Green Tea paths (see the module
    /// documentation for the ordering). Returns `true` if this call set it.
    #[inline(always)]
    pub(crate) fn try_mark_for_span_queue(&self, index: usize) -> bool {
        let mask = 1u64 << (index % 64);
        let word = &self.mark_bits[index / 64];
        if word.load(Relaxed) & mask != 0 {
            return false;
        }
        word.fetch_or(mask, SeqCst) & mask == 0
    }

    /// Set the scan bit of object `index`. Returns `true` if this call set it.
    #[inline(always)]
    pub(crate) fn try_set_scanned(&self, index: usize) -> bool {
        debug_assert!(index < SPAN_QUEUE_MAX_OBJS);
        let mask = 1u64 << (index % 64);
        let word = &self.scan_bits[index / 64];
        if word.load(SeqCst) & mask != 0 {
            return false;
        }
        word.fetch_or(mask, SeqCst) & mask == 0
    }

    pub fn is_scanned(&self, index: usize) -> bool {
        self.scan_bits[index / 64].load(SeqCst) & (1u64 << (index % 64)) != 0
    }

    /// Go: `spanSetScans`. For each word of the bitmaps, take the objects that are marked and not
    /// scanned, set their scan bits, and call `f` with each of their indices. Returns the number
    /// of objects. Call it after [`Self::release_ownership`].
    ///
    /// Unlike Go, the scan bits are set with the result of the atomic OR, so that two workers that
    /// scan the same span at the same time (after a release and a new acquire) never scan the
    /// same object twice.
    #[inline(always)]
    pub(crate) fn take_unscanned(&self, mut f: impl FnMut(usize)) -> usize {
        let words = self.nelems().div_ceil(64);
        debug_assert!(words <= SPAN_QUEUE_BITMAP_WORDS);
        let mut count = 0;
        for w in 0..words {
            // Marks first: an object allocated black sets its scan bit before its mark bit.
            let marks = self.mark_bits[w].load(SeqCst);
            let scans = self.scan_bits[w].load(SeqCst);
            let mut to_grey = marks & !scans;
            if to_grey == 0 {
                continue;
            }
            let old = self.scan_bits[w].fetch_or(to_grey, SeqCst);
            to_grey &= !old;
            count += to_grey.count_ones() as usize;
            while to_grey != 0 {
                let bit = to_grey.trailing_zeros() as usize;
                to_grey &= to_grey - 1;
                f(w * 64 + bit);
            }
        }
        count
    }

    /// Objects that are marked but have no scan bit. Zero at the end of marking, if every marked
    /// object went through the span queue, the object queue or allocate-black.
    pub(crate) fn count_marked_unscanned(&self) -> usize {
        let words = self.nelems().div_ceil(64).min(SPAN_QUEUE_BITMAP_WORDS);
        (0..words)
            .map(|w| {
                (self.mark_bits[w].load(SeqCst) & !self.scan_bits[w].load(SeqCst)).count_ones()
                    as usize
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::Address;

    fn new_span(size: usize) -> Span {
        let s = Span::new();
        s.init(
            unsafe { Address::from_usize(0x1000_0000) },
            SpanClass::new(size_to_class(size), false),
            4,
            false,
        );
        assert!(s.spanclass().uses_span_queue());
        s
    }

    /// Go's ownership protocol: the first marker acquires (one mark), later markers do not, and
    /// the owner's release reports whether more objects were marked.
    #[test]
    fn ownership() {
        let s = new_span(16);
        assert!(s.try_acquire());
        assert_eq!(s.owned.load(SeqCst), SPAN_SCAN_ONE_MARK);
        assert!(!s.try_acquire());
        assert!(!s.try_acquire());
        assert_eq!(
            s.release_ownership(),
            SPAN_SCAN_ONE_MARK | SPAN_SCAN_MANY_MARK
        );
        assert!(!s.is_owned());
        assert!(s.try_acquire());
        assert_eq!(s.release_ownership(), SPAN_SCAN_ONE_MARK);
    }

    /// `marks &^ scans`, once.
    #[test]
    fn take_unscanned_once() {
        let s = new_span(16);
        let n = s.nelems();
        assert_eq!(n, 512);
        for i in (0..n).step_by(5) {
            assert!(s.try_mark_for_span_queue(i));
        }
        // Allocated black: scan bit first, then mark bit. Never scanned.
        assert!(s.try_set_scanned(1));
        assert!(s.try_mark_for_span_queue(1));
        let mut got = vec![];
        assert_eq!(s.take_unscanned(|i| got.push(i)), n.div_ceil(5));
        assert_eq!(got, (0..n).step_by(5).collect::<Vec<_>>());
        assert_eq!(s.take_unscanned(|_| panic!("scanned twice")), 0);
        assert_eq!(s.count_marked_unscanned(), 0);
        assert!(s.try_mark_for_span_queue(n - 1));
        assert_eq!(s.count_marked_unscanned(), 1);
        let mut got = vec![];
        s.take_unscanned(|i| got.push(i));
        assert_eq!(got, vec![n - 1]);
    }
}
