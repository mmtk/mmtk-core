//! `SpanAllocator`: a port of Go's per-P allocation cache (`mcache`, `runtime/mcache.go`) for
//! [`SpanSpace`].
//!
//! The allocator caches one span per span class. Allocation takes the next free object of the
//! cached span with a count-trailing-zeros over the span's `alloc_cache` (Go: `nextFreeFast`).
//! When the cache is exhausted it refills it from the span's alloc bits (`nextFreeIndex`), and
//! when the span is full it returns the span to the space and takes another (`mcache.refill`):
//! a partially-free swept span, else an unswept span that it sweeps itself, else a new span.

use std::sync::Arc;

use super::allocator::AllocatorContext;
use crate::policy::space::Space;
use crate::policy::spanspace::classes::size_to_class_aligned;
use crate::policy::spanspace::span::EMPTY_SPAN;
use crate::policy::spanspace::{Span, SpanClass, SpanSpace, NUM_SPAN_CLASSES};
use crate::util::alloc::allocator;
use crate::util::alloc::Allocator;
use crate::util::Address;
use crate::util::VMThread;
use crate::vm::VMBinding;
use std::sync::atomic::Ordering;

/// A span allocator (Go's `mcache`).
///
/// `#[repr(C)]` with the per-class span array first, so that a binding can generate an inline
/// fast path from [`crate::util::alloc::AllocatorInfo::Span`].
#[repr(C)]
pub struct SpanAllocator<VM: VMBinding> {
    /// The cached span of each span class (Go: `mcache.alloc`). A slot with no span points to
    /// [`EMPTY_SPAN`], which has no free objects (Go: `emptymspan`).
    pub alloc: [&'static Span; NUM_SPAN_CLASSES],
    /// [`VMThread`] associated with this allocator instance.
    pub tls: VMThread,
    /// Whether this allocator allocates in noscan spans (objects the GC never scans).
    noscan: bool,
    space: &'static SpanSpace<VM>,
    context: Arc<AllocatorContext<VM>>,
    /// Go's `gcAssistBytes`: allocation credit during marking, in bytes.  Refills debit it; a
    /// negative value means the mutator owes mark work (an assist).
    pub(crate) assist_credit: isize,
}

impl<VM: VMBinding> Allocator<VM> for SpanAllocator<VM> {
    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space
    }

    fn get_context(&self) -> &AllocatorContext<VM> {
        &self.context
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        crate::policy::spanspace::BYTES_IN_SPAN_PAGE
    }

    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        let (spc, pad) = self.span_class(size, align, offset);
        let span = self.alloc[spc.index()];
        if let Some(cell) = span.next_free_fast() {
            Self::zero_if_needed(span, cell);
            return Self::align_in_cell(span, cell, align, offset, pad);
        }
        self.alloc_slow(size, align, offset)
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        if self.assist_credit < 0 {
            // Go: `deductAssistCredit` in `mallocgc`.  Done before touching any span, because
            // the assist may block through a GC pause.
            self.space
                .call_assist_hook(self.tls, &mut self.assist_credit);
        }
        let (spc, pad) = self.span_class(size, align, offset);
        match self.next_free(spc) {
            Some(cell) => Self::align_in_cell(self.alloc[spc.index()], cell, align, offset, pad),
            None => Address::ZERO,
        }
    }

    fn on_mutator_destroy(&mut self) {
        self.flush();
    }
}

impl<VM: VMBinding> SpanAllocator<VM> {
    pub(crate) fn new(
        tls: VMThread,
        space: &'static SpanSpace<VM>,
        context: Arc<AllocatorContext<VM>>,
    ) -> Self {
        SpanAllocator {
            alloc: [&EMPTY_SPAN; NUM_SPAN_CLASSES],
            tls,
            noscan: false,
            space,
            context,
            assist_credit: 0,
        }
    }

    /// Make this allocator allocate in noscan spans (or not). Only allowed while it caches no
    /// span.
    pub fn set_noscan(&mut self, noscan: bool) {
        debug_assert!(self.alloc.iter().all(|s| std::ptr::eq(*s, &EMPTY_SPAN)));
        self.noscan = noscan;
    }

    pub fn is_noscan(&self) -> bool {
        self.noscan
    }

    /// The span class for an allocation request, and whether the object must be aligned within
    /// its cell. Without an offset, the class is chosen so that every cell is suitably aligned.
    /// With an offset (not used by Go or OpenJDK), the cell is padded as other MMTk allocators
    /// do.
    #[inline(always)]
    fn span_class(&self, size: usize, align: usize, offset: usize) -> (SpanClass, bool) {
        let pad = offset != 0 && align > VM::MIN_ALIGNMENT;
        let (size, align) = if pad {
            (
                allocator::get_maximum_aligned_size::<VM>(size, align),
                VM::MIN_ALIGNMENT,
            )
        } else {
            (size, align)
        };
        let class = size_to_class_aligned(size, align).unwrap_or_else(|| {
            panic!("SpanAllocator: no size class for size {size} align {align}")
        });
        (SpanClass::new(class, self.noscan), pad)
    }

    #[inline(always)]
    fn align_in_cell(
        span: &Span,
        cell: Address,
        align: usize,
        offset: usize,
        pad: bool,
    ) -> Address {
        if pad {
            // Green Tea cannot find padded objects from their cells.
            span.padded.store(true, Ordering::Relaxed);
            allocator::align_allocation::<VM>(cell, align, offset)
        } else {
            cell
        }
    }

    #[inline(always)]
    fn zero_if_needed(span: &Span, addr: Address) {
        // Go: `mallocgc` zeroes the object if `span.needzero` (fresh pages are already zero).
        if span.needzero() {
            crate::util::memory::zero(addr, span.elemsize());
        }
    }

    /// Go: `mcache.nextFree`. Allocate from the cached span, refilling it if it is full.
    fn next_free(&mut self, spc: SpanClass) -> Option<Address> {
        let mut span = self.alloc[spc.index()];
        let mut free_index = span.next_free_index();
        if free_index == span.nelems() {
            // The span is full.
            debug_assert_eq!(span.alloc_count(), span.nelems());
            if !self.refill(spc) {
                return None;
            }
            span = self.alloc[spc.index()];
            free_index = span.next_free_index();
        }
        assert!(free_index < span.nelems(), "freeIndex is not valid");
        let addr = span.object_address(free_index);
        let count = span.alloc_count.fetch_add(1, Ordering::Relaxed) + 1;
        debug_assert!(count <= span.nelems());
        Self::zero_if_needed(span, addr);
        Some(addr)
    }

    /// Go: `mcache.refill`. Return the current span of class `spc` to the space and cache a new
    /// span with free objects. Returns false if the space could not provide one (a GC may have
    /// happened; the caller retries).
    fn refill(&mut self, spc: SpanClass) -> bool {
        let s = self.alloc[spc.index()];
        if !std::ptr::eq(s, &EMPTY_SPAN) {
            assert_eq!(
                s.sweepgen(),
                self.space.sweepgen().wrapping_add(3),
                "bad sweepgen in refill"
            );
            self.space.uncache_span(s);
            self.alloc[spc.index()] = &EMPTY_SPAN;
        }

        // Get a new cached span from the central sets.
        let Some(s) = self
            .space
            .cache_span(self.tls, spc, self.context.get_alloc_options())
        else {
            return false;
        };
        debug_assert!(s.alloc_count() < s.nelems(), "span has no free space");
        self.alloc[spc.index()] = s;

        // The trigger point (Go checks the GC trigger after a refill), and later the assist
        // debit point.
        let charged = s.bytes() - s.alloc_count() * s.elemsize();
        if self.space.is_marking() {
            self.assist_credit -= charged as isize;
        }
        self.space.call_refill_hook(self.tls, charged);
        true
    }

    /// Return every cached span to the space (Go: `mcache.releaseAll`, as called by
    /// `mcache.prepareForSweep` and `freemcache`). Spans cached before the last
    /// [`SpanSpace::release_after_mark`] are stale; they are swept here (Go: `uncacheSpan`).
    pub fn flush(&mut self) {
        for spc in 0..NUM_SPAN_CLASSES {
            let s = self.alloc[spc];
            if !std::ptr::eq(s, &EMPTY_SPAN) {
                self.space.uncache_span(s);
                self.alloc[spc] = &EMPTY_SPAN;
            }
        }
    }

    /// Called at the start of a GC (`Prepare`). Go does not flush mcaches when marking starts:
    /// objects in cached spans are marked like any others, and the spans stay cached (at
    /// `h + 3`). This does nothing.
    pub fn prepare(&mut self) {
        // Go: `gcResetMarkState` zeroes every goroutine's `gcAssistBytes` at the start of a
        // cycle.
        self.assist_credit = 0;
    }

    /// Called when a GC releases mutators, after [`SpanSpace::release_after_mark`] (Go: every
    /// P's `mcache.prepareForSweep` after mark termination). Returns and sweeps every cached
    /// span.
    pub fn release(&mut self) {
        self.flush();
    }
}
