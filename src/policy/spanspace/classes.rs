//! Size classes, span classes and the page constants of [`super::SpanSpace`].
//!
//! The tables themselves are generated from Go (see [`super::sizeclasses`]). This module adds
//! the lookups that Go's runtime performs on them (`runtime/malloc.go`, `runtime/mheap.go`).

use super::sizeclasses::*;

/// Log of the SpanSpace page size (Go's `pageShift`). A SpanSpace page is 8 KiB, i.e. two MMTk
/// pages.
pub const LOG_BYTES_IN_SPAN_PAGE: usize = PAGE_SHIFT;
/// The SpanSpace page size (Go's `pageSize`).
pub const BYTES_IN_SPAN_PAGE: usize = 1 << LOG_BYTES_IN_SPAN_PAGE;

/// The number of span classes: each size class has a scan and a noscan span class
/// (Go's `numSpanClasses`, `runtime/mheap.go`).
pub const NUM_SPAN_CLASSES: usize = NUM_SIZE_CLASSES << 1;

/// Words in a per-span bitmap (alloc bits or mark bits): one bit per object.
pub const BITMAP_WORDS: usize = MAX_OBJS_PER_SPAN.div_ceil(64);

/// The smallest object size whose spans use Green Tea's span queue (Go:
/// `gcUsesSpanInlineMarkBits`, `size >= 16`).
pub const SPAN_QUEUE_MIN_SIZE: usize = 16;
/// The largest object size whose spans use Green Tea's span queue (Go: `heapBitsInSpan`,
/// `minSizeForMallocHeader` = 512 B on 64-bit). These classes have one-page spans.
pub const SPAN_QUEUE_MAX_SIZE: usize = 512;
/// The most objects in a span that uses the span queue.
pub const SPAN_QUEUE_MAX_OBJS: usize = BYTES_IN_SPAN_PAGE / SPAN_QUEUE_MIN_SIZE;
/// Words in a span's scan bitmap (Green Tea).
pub const SPAN_QUEUE_BITMAP_WORDS: usize = SPAN_QUEUE_MAX_OBJS.div_ceil(64);

/// A span class: `size_class << 1 | noscan` (Go's `spanClass`, `runtime/mheap.go:573-592`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpanClass(pub u8);

impl SpanClass {
    /// Go: `makeSpanClass`.
    pub const fn new(size_class: usize, noscan: bool) -> Self {
        debug_assert!(size_class < NUM_SIZE_CLASSES);
        SpanClass(((size_class << 1) | (noscan as usize)) as u8)
    }

    /// Go: `spanClass.sizeclass`.
    pub const fn size_class(self) -> usize {
        (self.0 >> 1) as usize
    }

    /// Go: `spanClass.noscan`.
    pub const fn noscan(self) -> bool {
        self.0 & 1 != 0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Object size of this span class.
    pub const fn elem_size(self) -> usize {
        SIZE_CLASS_TO_SIZE[self.size_class()] as usize
    }

    /// Span size of this span class, in SpanSpace pages.
    pub const fn npages(self) -> usize {
        SIZE_CLASS_TO_NPAGES[self.size_class()] as usize
    }

    /// Number of objects in a span of this class (Go: `nelems` in `initSpan`, without Green Tea's
    /// inline mark bits).
    pub const fn nelems(self) -> usize {
        (self.npages() << LOG_BYTES_IN_SPAN_PAGE) / self.elem_size()
    }

    /// Whether spans of this class are scanned span-at-a-time by Green Tea (Go:
    /// `gcUsesSpanInlineMarkBits`, restricted to scan spans, since noscan spans are never
    /// scanned). These are the one-page spans of 16-512 B objects.
    pub const fn uses_span_queue(self) -> bool {
        let size = self.elem_size();
        !self.noscan() && size >= SPAN_QUEUE_MIN_SIZE && size <= SPAN_QUEUE_MAX_SIZE
    }

    /// The divide magic for this class (Go: `gc.SizeClassToDivMagic`).
    pub const fn div_magic(self) -> u32 {
        SIZE_CLASS_TO_DIV_MAGIC[self.size_class()]
    }
}

/// Go: `runtime/malloc.go` (`mallocgc*`): the size class of a small allocation of `size` bytes.
/// `size` must be in `1..=MAX_SMALL_SIZE`.
pub fn size_to_class(size: usize) -> usize {
    debug_assert!(size > 0 && size <= MAX_SMALL_SIZE);
    if size <= SMALL_SIZE_MAX - 8 {
        SIZE_TO_SIZE_CLASS8[size.div_ceil(SMALL_SIZE_DIV)] as usize
    } else {
        // For sizes in (SMALL_SIZE_MAX - 8, SMALL_SIZE_MAX], `size - SMALL_SIZE_MAX` wraps around
        // in Go, and so does `divRoundUp`, giving index 0. Do the same with wrapping arithmetic.
        let n = size.wrapping_sub(SMALL_SIZE_MAX);
        let index = n.wrapping_add(LARGE_SIZE_DIV - 1) / LARGE_SIZE_DIV;
        SIZE_TO_SIZE_CLASS128[index] as usize
    }
}

/// The size class for an allocation of `size` bytes aligned to `align`. Objects start at
/// `span_base + i * elem_size` and spans are 8 KiB aligned, so a class satisfies an alignment of
/// up to 8 KiB iff its object size is a multiple of it. Go only needs `MIN_HEAP_ALIGN`; larger
/// alignments (rare in Java) take the next class whose size is a multiple of `align`.
/// Returns `None` if no small class fits.
pub fn size_to_class_aligned(size: usize, align: usize) -> Option<usize> {
    if size == 0 || size > MAX_SMALL_SIZE {
        return None;
    }
    let mut class = size_to_class(size);
    if align <= MIN_HEAP_ALIGN {
        return Some(class);
    }
    if align > BYTES_IN_SPAN_PAGE {
        return None;
    }
    while class < NUM_SIZE_CLASSES {
        if SIZE_CLASS_TO_SIZE[class] as usize % align == 0 {
            return Some(class);
        }
        class += 1;
    }
    None
}

/// Object index of `offset` bytes into a span with the given divide magic (Go:
/// `mspan.divideByElemSize`, `runtime/mbitmap.go:1211`).
#[inline(always)]
pub const fn divide_by_elem_size(offset: usize, div_magic: u32) -> usize {
    ((offset as u64 * div_magic as u64) >> 32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go's `mksizeclasses.go` guarantees: each size class wastes at most 12.5% to rounding up and
    /// at most 12.5% to the span tail, for a combined worst case under 26.6% (except for the
    /// smallest classes, where alignment forces higher waste). Recompute it from the table.
    #[test]
    fn waste_bounds() {
        let mut prev = 0usize;
        for class in 1..NUM_SIZE_CLASSES {
            let size = SIZE_CLASS_TO_SIZE[class] as usize;
            let span = SIZE_CLASS_TO_NPAGES[class] as usize * BYTES_IN_SPAN_PAGE;
            let objects = span / size;
            let tail = span - objects * size;
            // Tail waste below 1/8 of the span (the `allocsize%size > allocsize/8` loop).
            assert!(
                tail <= span / 8,
                "class {class}: tail waste {tail} of {span}"
            );
            // Worst case: every object one byte larger than the previous class.
            let max_waste = ((size - prev - 1) * objects + tail) as f64 / span as f64;
            if size >= 128 {
                assert!(max_waste < 0.17, "class {class}: max waste {max_waste}");
            }
            assert!(max_waste < 0.875 + 1e-9);
            assert!(objects <= MAX_OBJS_PER_SPAN);
            assert!(SIZE_CLASS_TO_NPAGES[class] as usize <= MAX_SIZE_CLASS_NPAGES);
            assert!(size > prev, "sizes increase");
            assert_eq!(size % MIN_HEAP_ALIGN, 0);
            prev = size;
        }
        assert_eq!(prev, MAX_SMALL_SIZE);
    }

    /// `div_magic` must give the exact object index for every byte offset of every span
    /// (Go: `computeDivMagic`'s brute-force check in `mksizeclasses.go`).
    #[test]
    fn div_magic_exact() {
        for class in 1..NUM_SIZE_CLASSES {
            let size = SIZE_CLASS_TO_SIZE[class] as usize;
            let magic = SIZE_CLASS_TO_DIV_MAGIC[class];
            let max = SIZE_CLASS_TO_NPAGES[class] as usize * BYTES_IN_SPAN_PAGE;
            for n in 0..=max {
                assert_eq!(
                    divide_by_elem_size(n, magic),
                    n / size,
                    "class {class} n {n}"
                );
            }
        }
    }

    /// The lookup tables return the smallest class that fits.
    #[test]
    fn size_to_class_smallest_fit() {
        for size in 1..=MAX_SMALL_SIZE {
            let c = size_to_class(size);
            assert!(SIZE_CLASS_TO_SIZE[c] as usize >= size, "size {size}");
            assert!(c == 1 || (SIZE_CLASS_TO_SIZE[c - 1] as usize) < size);
        }
    }

    #[test]
    fn aligned_classes() {
        for align in [8usize, 16, 32, 64, 128, 256, 4096, 8192] {
            for size in (8..=MAX_SMALL_SIZE).step_by(8) {
                if let Some(c) = size_to_class_aligned(size, align) {
                    let s = SIZE_CLASS_TO_SIZE[c] as usize;
                    assert!(s >= size && s % align.max(8) == 0);
                }
            }
        }
        assert_eq!(size_to_class_aligned(MAX_SMALL_SIZE + 1, 8), None);
        assert_eq!(size_to_class_aligned(16, 16384), None);
    }

    #[test]
    fn span_queue_classes_are_one_page() {
        for class in 1..NUM_SIZE_CLASSES {
            for noscan in [false, true] {
                let spc = SpanClass::new(class, noscan);
                if spc.uses_span_queue() {
                    assert_eq!(spc.npages(), 1, "class {class}");
                    assert!(spc.nelems() <= SPAN_QUEUE_MAX_OBJS);
                }
            }
        }
        assert!(SpanClass::new(size_to_class(16), false).uses_span_queue());
        assert!(SpanClass::new(size_to_class(512), false).uses_span_queue());
        assert!(!SpanClass::new(size_to_class(513), false).uses_span_queue());
        assert!(!SpanClass::new(size_to_class(8), false).uses_span_queue());
        assert!(!SpanClass::new(size_to_class(64), true).uses_span_queue());
    }

    #[test]
    fn span_class_arithmetic() {
        for class in 1..NUM_SIZE_CLASSES {
            for noscan in [false, true] {
                let spc = SpanClass::new(class, noscan);
                assert_eq!(spc.size_class(), class);
                assert_eq!(spc.noscan(), noscan);
                assert!(spc.index() < NUM_SPAN_CLASSES);
                assert!(spc.nelems() >= 1 && spc.nelems() <= MAX_OBJS_PER_SPAN);
                assert!(spc.nelems() <= BITMAP_WORDS * 64);
            }
        }
        assert_eq!(NUM_SPAN_CLASSES, 136);
    }
}
