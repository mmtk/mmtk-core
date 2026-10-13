//! `SpanSpace`: a port of Go's span heap (design: `docs/design/go-gc-port.md` §3.5, §5.3-§5.5).
//!
//! The heap is organised as Go organises it:
//!
//! - **Size classes.** Go's 67 small size classes up to 32 KiB, with Go's span sizes, generated
//!   from Go's own generator ([`sizeclasses`], [`classes`]). Each size class has a scan and a
//!   noscan span class.
//! - **Pages and regions.** The space acquires 128 KiB page regions from MMTk (`Space::acquire`) and its
//!   page allocator ([`pages`]) hands out runs of 8 KiB pages within them, address-ordered first
//!   fit.
//! - **Spans.** A span is 1-10 pages of objects of one span class. Its descriptor ([`span`]) lives
//!   off-heap in an index-stable table and holds the alloc and mark bitmaps, indexed by object.
//!   A page map (side metadata, one `u32` span id per page) finds the span of an address, and a
//!   magic-number divide finds the object index (§5.13).
//! - **Central.** Per span class, partial/full × swept/unswept span sets ([`central`]).
//! - **Allocation.** [`crate::util::alloc::SpanAllocator`] is Go's `mcache`: one cached span per
//!   span class and a count-trailing-zeros fast path over the span's `alloc_cache`.
//! - **Green Tea.** While marking concurrently, small objects' spans are queued and scanned
//!   span-at-a-time ([`span_queue`], design §5.12), unless the option `concurrent_span_green_tea`
//!   is off.
//! - **Mark bits and sweeping.** Marks are set per object in the span's mark bitmap. Sweeping
//!   turns the mark bits into the alloc bits and clears the mark bits, so marks are never cleared
//!   at Prepare. Sweeping follows Go's `sweepgen` protocol ([`sweep`]), so allocators, sweep
//!   packets and `sweep_one` can sweep at the same time, and lazily-unswept spans can never be
//!   swept against the marks of a cycle in progress (design §4 gap 1).
//!
//! # Plan protocol
//!
//! | When | Call |
//! |---|---|
//! | Pause that starts marking (Prepare) | [`SpanSpace::prepare_initial_mark`] |
//! | While marking | [`SpanSpace::trace_object`], or [`SpanSpace::trace_object_for_span_queue`] and [`SpanSpace::scan_span`] (Green Tea); `set_allocate_as_live(true)` for concurrent marking |
//! | Pause that ends marking (Release) | `set_allocate_as_live(false)`, [`SpanSpace::release_after_mark`], then [`SpanSpace::schedule_sweep`] |
//! | Mutator release (after the plan's release) | [`crate::util::alloc::SpanAllocator::release`] |
//! | Before triggering a cycle | [`SpanSpace::has_unswept`], [`SpanSpace::sweep_one`] |
//! | Pacer inputs | [`SpanSpace::heap_live`], [`SpanSpace::bytes_marked`], [`SpanSpace::take_bytes_marked`], [`SpanSpace::set_refill_hook`] |
//!
//! With the `vo_bit` feature (which requires dead objects' VO bits to be cleared by the end of
//! each GC), the plan must sweep eagerly in the pause (`schedule_sweep(Release)`).
//!
//! Deviations from Go's code are listed in `docs/design/concurrent-span-go-deviations.md`
//! (entries SP1-SP12) and decisions D30-D38 in `docs/design/concurrent-span-decisions.md`.

// Parts of the plan-facing API (pacer inputs, sweep control) are only used by ConcurrentSpan's
// concurrent phases, which are not implemented yet.
#![allow(dead_code)]

pub mod central;
pub mod classes;
mod global;
pub mod pages;
#[rustfmt::skip]
#[allow(clippy::all, dead_code)]
pub mod sizeclasses;
pub mod span;
pub mod span_queue;
pub mod sweep;

pub use classes::{SpanClass, BYTES_IN_SPAN_PAGE, LOG_BYTES_IN_SPAN_PAGE, NUM_SPAN_CLASSES};
#[allow(unused_imports)]
pub use global::{RefillHook, SpanQueueTrace, SpanSpace};
pub use sizeclasses::MAX_SMALL_SIZE;
pub use span::Span;
#[allow(unused_imports)]
pub use span_queue::SpanWork;

/// The largest object `SpanSpace` allocates (Go's `maxSmallSize`). Larger objects go to the LOS.
pub const MAX_OBJECT_SIZE: usize = MAX_SMALL_SIZE;
