//! Span descriptors (Go's `mspan`, `runtime/mheap.go:422-516`) and the index-stable table that
//! holds them.
//!
//! A span is a run of 1-10 SpanSpace pages (8 KiB each) holding objects of one span class.
//! Allocation state follows Go:
//!
//! - `alloc_bits` has one bit per object, set for objects that were live after the last sweep.
//! - `freeindex` is the index from which the allocator looks for a free object. Every object below
//!   `freeindex` is allocated.
//! - `alloc_cache` holds the complement of the 64 alloc bits starting at `freeindex & !63`,
//!   shifted so that bit 0 corresponds to `freeindex`. The allocator finds the next free object
//!   with a count-trailing-zeros (`nextFreeFast`).
//! - `mark_bits` has one bit per object, set by tracing. Sweeping turns the mark bits into the
//!   alloc bits and clears the mark bits (Go swaps two `gcBits` arenas instead, see deviation
//!   SP1 in `docs/design/concurrent-span-go-deviations.md`).
//!
//! All fields are atomics so that the descriptor can be shared safely. Fields owned by the
//! allocator that caches the span (or by the sweeper that owns it) are accessed with `Relaxed`
//! ordering; ownership is transferred through `sweepgen` and the central lists, which use
//! stronger orderings.

use super::classes::*;
use crate::util::Address;
use std::sync::atomic::Ordering::*;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicU8, AtomicUsize};
use std::sync::Mutex;

/// The state of a span descriptor (Go: `mSpanState`).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpanState {
    /// Not in use. The descriptor is on the free list of the span table.
    Dead = 0,
    /// In use for small objects.
    InUse = 1,
}

/// A span id. Id 0 is never allocated: a zero entry in the page map means "no span".
pub type SpanId = u32;

/// A span descriptor. `#[repr(C)]`, and the fields used by the allocation fast path come first, so
/// that a binding can generate an inline fast path from their offsets (see
/// [`crate::util::alloc::AllocatorInfo::Span`]).
#[repr(C)]
pub struct Span {
    /// Inverted alloc bits from `freeindex` (Go: `allocCache`).
    pub(crate) alloc_cache: AtomicU64,
    /// Next object index to look at (Go: `freeindex`).
    pub(crate) freeindex: AtomicUsize,
    /// Number of objects in the span (Go: `nelems`).
    pub(crate) nelems: AtomicUsize,
    /// Object size (Go: `elemsize`).
    pub(crate) elemsize: AtomicUsize,
    /// Start address (Go: `startAddr`).
    pub(crate) start: AtomicUsize,
    /// Number of allocated objects (Go: `allocCount`).
    pub(crate) alloc_count: AtomicUsize,
    /// `alloc_count` when the span was cached (Go: `allocCountBeforeCache`).
    pub(crate) alloc_count_before_cache: AtomicUsize,
    /// Number of SpanSpace pages (Go: `npages`).
    pub(crate) npages: AtomicUsize,
    /// Go: `divMul`.
    pub(crate) div_magic: AtomicU32,
    /// Sweep generation, relative to the space's `sweepgen` `h` (Go, `runtime/mheap.go:494-500`):
    /// `h-2`: needs sweeping; `h-1`: being swept; `h`: swept and ready to use;
    /// `h+1`: cached before sweep began, still cached and needs sweeping; `h+3`: swept, then
    /// cached, still cached.
    pub(crate) sweepgen: AtomicU32,
    /// Go: `spanclass`.
    pub(crate) spanclass: AtomicU8,
    /// Go: `state`.
    pub(crate) state: AtomicU8,
    /// Whether free objects may hold stale data and must be zeroed on allocation (Go: `needzero`).
    pub(crate) needzero: AtomicBool,
    /// One bit per object: live after the last sweep (Go: `allocBits`).
    pub(crate) alloc_bits: [AtomicU64; BITMAP_WORDS],
    /// One bit per object: marked in the current cycle (Go: `gcmarkBits`).
    pub(crate) mark_bits: [AtomicU64; BITMAP_WORDS],
}

impl Default for Span {
    fn default() -> Self {
        Self::new()
    }
}

impl Span {
    pub const fn new() -> Self {
        Span {
            alloc_cache: AtomicU64::new(0),
            freeindex: AtomicUsize::new(0),
            nelems: AtomicUsize::new(0),
            elemsize: AtomicUsize::new(0),
            start: AtomicUsize::new(0),
            alloc_count: AtomicUsize::new(0),
            alloc_count_before_cache: AtomicUsize::new(0),
            npages: AtomicUsize::new(0),
            div_magic: AtomicU32::new(0),
            sweepgen: AtomicU32::new(0),
            spanclass: AtomicU8::new(0),
            state: AtomicU8::new(SpanState::Dead as u8),
            needzero: AtomicBool::new(false),
            alloc_bits: [const { AtomicU64::new(0) }; BITMAP_WORDS],
            mark_bits: [const { AtomicU64::new(0) }; BITMAP_WORDS],
        }
    }

    /// Initialise a descriptor for a new span (Go: `mspan.init` + `mheap.initSpan`). The new
    /// span has no allocated and no marked objects.
    pub(crate) fn init(&self, start: Address, spanclass: SpanClass, sweepgen: u32, needzero: bool) {
        debug_assert!(start.is_aligned_to(BYTES_IN_SPAN_PAGE));
        self.start.store(start.as_usize(), Relaxed);
        self.npages.store(spanclass.npages(), Relaxed);
        self.spanclass.store(spanclass.0, Relaxed);
        self.elemsize.store(spanclass.elem_size(), Relaxed);
        self.nelems.store(spanclass.nelems(), Relaxed);
        self.div_magic.store(spanclass.div_magic(), Relaxed);
        self.freeindex.store(0, Relaxed);
        self.alloc_count.store(0, Relaxed);
        self.alloc_count_before_cache.store(0, Relaxed);
        self.needzero.store(needzero, Relaxed);
        for w in 0..BITMAP_WORDS {
            self.alloc_bits[w].store(0, Relaxed);
            self.mark_bits[w].store(0, Relaxed);
        }
        self.refill_alloc_cache(0);
        self.sweepgen.store(sweepgen, Relaxed);
        self.state.store(SpanState::InUse as u8, Release);
    }

    pub fn start(&self) -> Address {
        unsafe { Address::from_usize(self.start.load(Relaxed)) }
    }
    pub fn npages(&self) -> usize {
        self.npages.load(Relaxed)
    }
    pub fn bytes(&self) -> usize {
        self.npages() << LOG_BYTES_IN_SPAN_PAGE
    }
    pub fn end(&self) -> Address {
        self.start() + self.bytes()
    }
    pub fn nelems(&self) -> usize {
        self.nelems.load(Relaxed)
    }
    pub fn elemsize(&self) -> usize {
        self.elemsize.load(Relaxed)
    }
    pub fn spanclass(&self) -> SpanClass {
        SpanClass(self.spanclass.load(Relaxed))
    }
    pub fn alloc_count(&self) -> usize {
        self.alloc_count.load(Relaxed)
    }
    pub fn freeindex(&self) -> usize {
        self.freeindex.load(Relaxed)
    }
    pub fn sweepgen(&self) -> u32 {
        self.sweepgen.load(Acquire)
    }
    pub fn state(&self) -> SpanState {
        if self.state.load(Acquire) == SpanState::InUse as u8 {
            SpanState::InUse
        } else {
            SpanState::Dead
        }
    }
    pub fn is_in_use(&self) -> bool {
        self.state() == SpanState::InUse
    }
    pub fn needzero(&self) -> bool {
        self.needzero.load(Relaxed)
    }

    /// Address of object `index`.
    pub fn object_address(&self, index: usize) -> Address {
        self.start() + index * self.elemsize()
    }

    /// Object index of `addr`, which may point anywhere inside the object (Go: `mspan.objIndex`).
    #[inline(always)]
    pub fn obj_index(&self, addr: Address) -> usize {
        divide_by_elem_size(addr - self.start(), self.div_magic.load(Relaxed))
    }

    /// Go: `mspan.refillAllocCache`. `word` is the index of the 64-bit word of `alloc_bits`
    /// (Go uses a byte index that is always a multiple of 8).
    pub(crate) fn refill_alloc_cache(&self, word: usize) {
        let bits = if word < BITMAP_WORDS {
            self.alloc_bits[word].load(Relaxed)
        } else {
            u64::MAX
        };
        self.alloc_cache.store(!bits, Relaxed);
    }

    /// Go: `nextFreeFast` (`runtime/malloc.go:977`). Returns the address of a free object if
    /// one is available in `alloc_cache`, without refilling the cache.
    #[inline(always)]
    pub(crate) fn next_free_fast(&self) -> Option<Address> {
        let cache = self.alloc_cache.load(Relaxed);
        let the_bit = cache.trailing_zeros() as usize;
        if the_bit < 64 {
            let freeindex = self.freeindex.load(Relaxed);
            let result = freeindex + the_bit;
            let nelems = self.nelems.load(Relaxed);
            if result < nelems {
                let freeidx = result + 1;
                if freeidx % 64 == 0 && freeidx != nelems {
                    return None;
                }
                self.alloc_cache.store(cache >> (the_bit + 1), Relaxed);
                self.freeindex.store(freeidx, Relaxed);
                self.alloc_count.fetch_add(1, Relaxed);
                return Some(self.object_address(result));
            }
        }
        None
    }

    /// Go: `mspan.nextFreeIndex` (`runtime/mbitmap.go:1115`). Returns the index of the next free
    /// object at or after `freeindex`, or `nelems` if there is none. Does not count the
    /// allocation.
    pub(crate) fn next_free_index(&self) -> usize {
        let mut sfreeindex = self.freeindex.load(Relaxed);
        let snelems = self.nelems.load(Relaxed);
        if sfreeindex == snelems {
            return sfreeindex;
        }
        assert!(sfreeindex < snelems, "s.freeindex > s.nelems");

        let mut a_cache = self.alloc_cache.load(Relaxed);
        let mut bit_index = a_cache.trailing_zeros() as usize;
        while bit_index == 64 {
            // Move index to start of next cached bits.
            sfreeindex = (sfreeindex + 64) & !(64 - 1);
            if sfreeindex >= snelems {
                self.freeindex.store(snelems, Relaxed);
                return snelems;
            }
            // Refill alloc_cache with the next 64 alloc bits.
            self.refill_alloc_cache(sfreeindex / 64);
            a_cache = self.alloc_cache.load(Relaxed);
            bit_index = a_cache.trailing_zeros() as usize;
        }
        let result = sfreeindex + bit_index;
        if result >= snelems {
            self.freeindex.store(snelems, Relaxed);
            return snelems;
        }

        self.alloc_cache.store(a_cache >> (bit_index + 1), Relaxed);
        sfreeindex = result + 1;

        if sfreeindex % 64 == 0 && sfreeindex != snelems {
            // All bits of the cache have been used: refill it so that it corresponds to the bits
            // starting at the new freeindex.
            self.refill_alloc_cache(sfreeindex / 64);
        }
        self.freeindex.store(sfreeindex, Relaxed);
        result
    }

    /// Prepare `alloc_cache` for allocation from `freeindex` (the `havespan` part of Go's
    /// `mcentral.cacheSpan`).
    pub(crate) fn init_alloc_cache_at_freeindex(&self) {
        let freeindex = self.freeindex.load(Relaxed);
        self.refill_alloc_cache(freeindex / 64);
        let cache = self.alloc_cache.load(Relaxed);
        self.alloc_cache.store(cache >> (freeindex % 64), Relaxed);
    }

    /// Is object `index` allocated? (Go: `!mspan.isFree`.) Only meaningful for the owner of the
    /// span or when the owner is stopped.
    pub fn is_allocated(&self, index: usize) -> bool {
        index < self.freeindex.load(Relaxed) || bit_is_set(&self.alloc_bits, index)
    }

    pub fn is_marked(&self, index: usize) -> bool {
        bit_is_set(&self.mark_bits, index)
    }

    /// Atomically set the mark bit of object `index`. Returns `true` if this call set it.
    #[inline(always)]
    pub fn try_mark(&self, index: usize) -> bool {
        let mask = 1u64 << (index % 64);
        let word = &self.mark_bits[index / 64];
        if word.load(Relaxed) & mask != 0 {
            return false;
        }
        word.fetch_or(mask, Relaxed) & mask == 0
    }

    /// Number of marked objects (Go: `mspan.countAlloc`).
    pub(crate) fn count_marked(&self) -> usize {
        let words = self.nelems().div_ceil(64);
        (0..words)
            .map(|w| self.mark_bits[w].load(Relaxed).count_ones() as usize)
            .sum()
    }

    /// The bit work of sweeping (the middle of Go's `sweepLocked.sweep`,
    /// `runtime/mgcsweep.go:512`): the mark bits become the alloc bits, the mark bits are
    /// cleared, and the allocation index is reset. `on_freed` is called with the index of every
    /// object that was allocated and is now free. Returns `(nalloc, nfreed)`.
    pub(crate) fn sweep_bits(&self, mut on_freed: impl FnMut(usize)) -> (usize, usize) {
        let nelems = self.nelems();
        let words = nelems.div_ceil(64);
        let freeindex = self.freeindex();
        for w in 0..words {
            let mark = self.mark_bits[w].load(Relaxed);
            let alloc = self.alloc_bits[w].load(Relaxed);
            // Allocated: below freeindex, or allocated before the last sweep.
            let below_freeindex = mask_below(freeindex, w);
            let allocated = alloc | below_freeindex;
            // Zombies: marked but free (Go: `mspan.reportZombies`).
            debug_assert!(
                mark & !allocated == 0,
                "SpanSpace: marked free object(s) in span {} word {}: mark {:x} allocated {:x}",
                self.start(),
                w,
                mark,
                allocated
            );
            let mut freed = allocated & !mark;
            while freed != 0 {
                let bit = freed.trailing_zeros() as usize;
                let index = w * 64 + bit;
                if index < nelems {
                    on_freed(index);
                }
                freed &= freed - 1;
            }
        }
        let nalloc = self.count_marked();
        let alloc_count = self.alloc_count();
        assert!(
            nalloc <= alloc_count,
            "sweep increased allocation count: nalloc {nalloc} > alloc_count {alloc_count} (span {})",
            self.start()
        );
        let nfreed = alloc_count - nalloc;
        self.alloc_count.store(nalloc, Relaxed);
        self.freeindex.store(0, Relaxed);
        for w in 0..BITMAP_WORDS {
            let mark = self.mark_bits[w].swap(0, Relaxed);
            self.alloc_bits[w].store(mark, Relaxed);
        }
        self.refill_alloc_cache(0);
        (nalloc, nfreed)
    }
}

fn bit_is_set(bits: &[AtomicU64; BITMAP_WORDS], index: usize) -> bool {
    bits[index / 64].load(Relaxed) & (1u64 << (index % 64)) != 0
}

/// Bits of word `w` whose object index is below `freeindex`.
fn mask_below(freeindex: usize, w: usize) -> u64 {
    let lo = w * 64;
    if freeindex <= lo {
        0
    } else if freeindex >= lo + 64 {
        u64::MAX
    } else {
        (1u64 << (freeindex - lo)) - 1
    }
}

/// The empty span that an allocator's slot points to when it caches no span (Go: `emptymspan`).
/// `nelems == 0`, so the allocator never allocates from it and never writes to it.
pub static EMPTY_SPAN: Span = Span::new();

const LOG_SEGMENT_SPANS: usize = 10;
const SEGMENT_SPANS: usize = 1 << LOG_SEGMENT_SPANS;
/// Maximum number of segments. With 1024 descriptors per segment, this allows 2^26 spans, i.e.
/// at least 512 GiB of 8 KiB spans.
const MAX_SEGMENTS: usize = 1 << 16;

/// An index-stable, growable table of span descriptors (Go allocates `mspan`s with `fixalloc`).
/// Descriptors never move, so `&'static Span` references taken from a `'static` table stay valid.
/// Freed descriptors are reused.
pub struct SpanTable {
    segments: Box<[AtomicPtr<Span>]>,
    alloc: Mutex<SpanTableAlloc>,
    /// One past the highest id ever allocated. Readers can iterate `1..high_water`.
    high_water: AtomicU32,
}

struct SpanTableAlloc {
    next: SpanId,
    free: Vec<SpanId>,
}

impl Default for SpanTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SpanTable {
    pub fn new() -> Self {
        SpanTable {
            segments: (0..MAX_SEGMENTS)
                .map(|_| AtomicPtr::new(std::ptr::null_mut()))
                .collect(),
            alloc: Mutex::new(SpanTableAlloc {
                next: 1,
                free: vec![],
            }),
            high_water: AtomicU32::new(1),
        }
    }

    /// Allocate a descriptor id. The descriptor is `Dead` until the caller initialises it.
    pub fn alloc_id(&self) -> SpanId {
        let mut alloc = self.alloc.lock().unwrap();
        if let Some(id) = alloc.free.pop() {
            return id;
        }
        let id = alloc.next;
        let seg = id as usize >> LOG_SEGMENT_SPANS;
        assert!(seg < MAX_SEGMENTS, "SpanSpace: out of span descriptors");
        if self.segments[seg].load(Acquire).is_null() {
            let segment: Box<[Span]> = (0..SEGMENT_SPANS).map(|_| Span::new()).collect();
            let ptr = Box::into_raw(segment) as *mut Span;
            self.segments[seg].store(ptr, Release);
        }
        alloc.next += 1;
        self.high_water.store(alloc.next, Release);
        id
    }

    /// Return a descriptor id. The descriptor must already be `Dead`.
    pub fn free_id(&self, id: SpanId) {
        debug_assert!(!self.get(id).is_in_use());
        self.alloc.lock().unwrap().free.push(id);
    }

    /// Get the descriptor for an id that has been allocated.
    #[inline(always)]
    pub fn get(&self, id: SpanId) -> &Span {
        let seg = self.segments[id as usize >> LOG_SEGMENT_SPANS].load(Acquire);
        debug_assert!(!seg.is_null(), "span id {id} was never allocated");
        unsafe { &*seg.add(id as usize & (SEGMENT_SPANS - 1)) }
    }

    /// One past the highest id allocated so far.
    pub fn high_water(&self) -> SpanId {
        self.high_water.load(Acquire)
    }

    /// Iterate over all in-use spans (with their ids).
    pub fn iter_in_use(&self) -> impl Iterator<Item = (SpanId, &Span)> + '_ {
        (1..self.high_water())
            .map(move |id| (id, self.get(id)))
            .filter(|(_, s)| s.is_in_use())
    }
}

impl Drop for SpanTable {
    fn drop(&mut self) {
        for seg in self.segments.iter() {
            let ptr = seg.load(Relaxed);
            if !ptr.is_null() {
                let slice = std::ptr::slice_from_raw_parts_mut(ptr, SEGMENT_SPANS);
                drop(unsafe { Box::from_raw(slice) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::spanspace::sizeclasses::NUM_SIZE_CLASSES;

    fn new_span(class: usize) -> Span {
        let s = Span::new();
        // Any 8 KiB-aligned address works: these tests never touch the memory.
        s.init(
            unsafe { Address::from_usize(0x1000_0000) },
            SpanClass::new(class, false),
            4,
            false,
        );
        s
    }

    /// Allocate every object of a fresh span with the fast path plus `next_free_index`, as the
    /// allocator does, and check that each index is returned once, in order.
    #[test]
    fn allocate_whole_span() {
        for class in 1..NUM_SIZE_CLASSES {
            let s = new_span(class);
            let mut expected = 0;
            loop {
                let addr = match s.next_free_fast() {
                    Some(a) => a,
                    None => {
                        let i = s.next_free_index();
                        if i == s.nelems() {
                            break;
                        }
                        s.alloc_count.fetch_add(1, Relaxed);
                        s.object_address(i)
                    }
                };
                assert_eq!(s.obj_index(addr), expected, "class {class}");
                expected += 1;
            }
            assert_eq!(expected, s.nelems());
            assert_eq!(s.alloc_count(), s.nelems());
        }
    }

    /// Sweep: marked objects survive, unmarked allocated objects are freed, and allocation then
    /// reuses exactly the freed slots.
    #[test]
    fn sweep_then_reuse() {
        for class in [1usize, 2, 5, 17, 35, 67] {
            let s = new_span(class);
            let n = s.nelems();
            // Allocate everything.
            while s.next_free_index() != n {
                s.alloc_count.fetch_add(1, Relaxed);
            }
            // Mark every third object, and the object at every offset of its cell maps back.
            for i in (0..n).step_by(3) {
                let a = s.object_address(i);
                for off in [0, s.elemsize() / 2, s.elemsize() - 1] {
                    assert_eq!(s.obj_index(a + off), i);
                }
                assert!(s.try_mark(i));
                assert!(!s.try_mark(i));
            }
            let mut freed = vec![];
            let (nalloc, nfreed) = s.sweep_bits(|i| freed.push(i));
            assert_eq!(nalloc, n.div_ceil(3));
            assert_eq!(nfreed, n - nalloc);
            assert_eq!(freed.len(), nfreed);
            assert!(freed.iter().all(|i| i % 3 != 0));
            assert_eq!(s.count_marked(), 0);
            for i in 0..n {
                assert_eq!(s.is_allocated(i), i % 3 == 0);
            }
            // Reuse: allocation returns exactly the freed indices, in order.
            s.init_alloc_cache_at_freeindex();
            let mut got = vec![];
            loop {
                if let Some(a) = s.next_free_fast() {
                    got.push(s.obj_index(a));
                    continue;
                }
                let i = s.next_free_index();
                if i == n {
                    break;
                }
                s.alloc_count.fetch_add(1, Relaxed);
                got.push(i);
            }
            assert_eq!(got, freed);
            assert_eq!(s.alloc_count(), n);
        }
    }

    #[test]
    fn empty_span_never_allocates() {
        assert_eq!(EMPTY_SPAN.next_free_fast(), None);
        assert_eq!(EMPTY_SPAN.next_free_index(), 0);
        assert_eq!(EMPTY_SPAN.nelems(), 0);
    }

    #[test]
    fn span_table_ids_are_stable_and_reused() {
        let t = SpanTable::new();
        let a = t.alloc_id();
        let b = t.alloc_id();
        assert_eq!((a, b), (1, 2));
        let pa = t.get(a) as *const Span;
        for _ in 0..(SEGMENT_SPANS * 2) {
            t.alloc_id();
        }
        assert_eq!(t.get(a) as *const Span, pa);
        t.free_id(b);
        assert_eq!(t.alloc_id(), b);
        assert_eq!(t.iter_in_use().count(), 0);
    }
}
