//! The SpanSpace page allocator (Go's `pageAlloc`, `runtime/mpagealloc.go`).
//!
//! The space acquires memory from MMTk in *page regions* of [`BYTES_IN_PAGE_REGION`] (128 KiB),
//! and this allocator hands out runs of 8 KiB pages within those regions. Like Go's, it keeps an
//! in-use bitmap per region (Go's `pallocBits`, which covers a 4 MiB chunk) and does an
//! address-ordered first-fit search. Go accelerates the search with a radix tree of summaries;
//! this scans, in address order, only the regions that have a free page. A region whose pages are all free is handed back
//! to MMTk, so that other spaces can use it and MMTk's heap accounting sees it as free. See
//! deviation SP2 and decision D30 in `docs/design/`.
//!
//! The allocator also tracks which free pages may hold stale data (Go tracks this per arena with
//! `zeroedBase`, and per span with `needzero`).

use super::classes::LOG_BYTES_IN_SPAN_PAGE;
use crate::util::Address;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Log of the size of the unit in which SpanSpace acquires memory from MMTk.
pub const LOG_BYTES_IN_PAGE_REGION: usize = 17;
/// The size of the unit in which SpanSpace acquires memory from MMTk (128 KiB). It must hold the
/// largest span (10 pages) and divide an MMTk chunk. MMTk's free-list page resource keeps
/// equal-sized requests aligned to their size, so regions are naturally aligned.
pub const BYTES_IN_PAGE_REGION: usize = 1 << LOG_BYTES_IN_PAGE_REGION;
/// SpanSpace pages in a page region.
pub const SPAN_PAGES_IN_REGION: usize = BYTES_IN_PAGE_REGION >> LOG_BYTES_IN_SPAN_PAGE;

const_assert!(SPAN_PAGES_IN_REGION >= super::sizeclasses::MAX_SIZE_CLASS_NPAGES);
const_assert!(crate::util::heap::layout::vm_layout::BYTES_IN_CHUNK % BYTES_IN_PAGE_REGION == 0);

const WORDS: usize = SPAN_PAGES_IN_REGION.div_ceil(64);

#[derive(Debug)]
struct RegionPages {
    /// One bit per page: in use.
    in_use: [u64; WORDS],
    /// One bit per page: the page has been used since it was last known to be zero.
    dirty: [u64; WORDS],
    used: usize,
}

impl RegionPages {
    fn new(zeroed: bool) -> Self {
        RegionPages {
            in_use: [0; WORDS],
            dirty: if zeroed {
                [0; WORDS]
            } else {
                [u64::MAX; WORDS]
            },
            used: 0,
        }
    }

    fn get(bits: &[u64; WORDS], i: usize) -> bool {
        bits[i / 64] & (1 << (i % 64)) != 0
    }

    fn set_range(bits: &mut [u64; WORDS], from: usize, n: usize, value: bool) {
        for i in from..from + n {
            if value {
                bits[i / 64] |= 1 << (i % 64);
            } else {
                bits[i / 64] &= !(1 << (i % 64));
            }
        }
    }

    /// First fit: the lowest index of `npages` contiguous free pages (Go: `pallocBits.find`).
    fn find(&self, npages: usize) -> Option<usize> {
        let mut run = 0;
        for i in 0..SPAN_PAGES_IN_REGION {
            if Self::get(&self.in_use, i) {
                run = 0;
            } else {
                run += 1;
                if run == npages {
                    return Some(i + 1 - npages);
                }
            }
        }
        None
    }
}

/// Page allocator state, behind one lock (Go's `mheap_.lock` protects `pageAlloc` too).
#[derive(Default)]
pub struct PageAllocator {
    inner: Mutex<Inner>,
    /// Pages in use, over all regions.  Updated under the lock, read without it.
    used: AtomicUsize,
}

#[derive(Default)]
struct Inner {
    regions: BTreeMap<usize, RegionPages>,
    /// The regions with at least one free page, so that a nearly full heap is not scanned
    /// region by region.
    nonfull: BTreeSet<usize>,
}

/// The result of freeing pages.
#[derive(Debug, PartialEq, Eq)]
pub enum FreeResult {
    /// The region still has pages in use.
    Retained,
    /// All pages of the region are free. The region has been removed from the allocator and
    /// should be returned to MMTk.
    RegionEmpty(Address),
}

impl PageAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a page region acquired from MMTk. `zeroed` says whether its memory is known to be
    /// zero.
    pub fn add_region(&self, region: Address, zeroed: bool) {
        assert!(region.is_aligned_to(BYTES_IN_PAGE_REGION));
        let mut inner = self.inner.lock().unwrap();
        let old = inner
            .regions
            .insert(region.as_usize(), RegionPages::new(zeroed));
        debug_assert!(old.is_none(), "region {region} added twice");
        inner.nonfull.insert(region.as_usize());
    }

    /// Allocate `npages` contiguous pages, address-ordered first fit. Returns the start address
    /// and whether any of the pages may hold stale data (Go's `needzero`).
    pub fn alloc(&self, npages: usize) -> Option<(Address, bool)> {
        debug_assert!(npages > 0 && npages <= SPAN_PAGES_IN_REGION);
        let inner = &mut *self.inner.lock().unwrap();
        for &start in inner.nonfull.iter() {
            let pages = inner.regions.get_mut(&start).unwrap();
            if SPAN_PAGES_IN_REGION - pages.used < npages {
                continue;
            }
            if let Some(i) = pages.find(npages) {
                RegionPages::set_range(&mut pages.in_use, i, npages, true);
                let needzero = (i..i + npages).any(|p| RegionPages::get(&pages.dirty, p));
                RegionPages::set_range(&mut pages.dirty, i, npages, true);
                pages.used += npages;
                self.used.fetch_add(npages, Ordering::Relaxed);
                if pages.used == SPAN_PAGES_IN_REGION {
                    inner.nonfull.remove(&start);
                }
                let addr = unsafe { Address::from_usize(start) } + (i << LOG_BYTES_IN_SPAN_PAGE);
                return Some((addr, needzero));
            }
        }
        None
    }

    /// Free `npages` pages starting at `addr`.
    pub fn free(&self, addr: Address, npages: usize) -> FreeResult {
        let region = addr.align_down(BYTES_IN_PAGE_REGION);
        let i = (addr - region) >> LOG_BYTES_IN_SPAN_PAGE;
        let inner = &mut *self.inner.lock().unwrap();
        let pages = inner
            .regions
            .get_mut(&region.as_usize())
            .unwrap_or_else(|| panic!("freeing pages {addr} in an unknown region"));
        for p in i..i + npages {
            debug_assert!(
                RegionPages::get(&pages.in_use, p),
                "double free of page {p}"
            );
        }
        RegionPages::set_range(&mut pages.in_use, i, npages, false);
        pages.used -= npages;
        self.used.fetch_sub(npages, Ordering::Relaxed);
        if pages.used == 0 {
            inner.regions.remove(&region.as_usize());
            inner.nonfull.remove(&region.as_usize());
            FreeResult::RegionEmpty(region)
        } else {
            inner.nonfull.insert(region.as_usize());
            FreeResult::Retained
        }
    }

    /// All page regions currently owned by the allocator.
    pub fn regions(&self) -> Vec<Address> {
        self.inner
            .lock()
            .unwrap()
            .regions
            .keys()
            .map(|&a| unsafe { Address::from_usize(a) })
            .collect()
    }

    /// Pages in use, over all regions.
    pub fn used_pages(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(n: usize) -> Address {
        unsafe { Address::from_usize((n + 256) * BYTES_IN_PAGE_REGION) }
    }

    #[test]
    fn first_fit_and_free() {
        let pa = PageAllocator::new();
        assert_eq!(pa.alloc(1), None);
        pa.add_region(region(1), true);
        pa.add_region(region(0), true);
        // Address order: the lower region first.
        let (a, z) = pa.alloc(1).unwrap();
        assert_eq!(a, region(0));
        assert!(!z);
        let (b, _) = pa.alloc(3).unwrap();
        assert_eq!(b, region(0) + 8192usize);
        let (c, _) = pa.alloc(1).unwrap();
        assert_eq!(c, region(0) + 4 * 8192usize);
        // Free the 3-page run; a 2-page request fits there (first fit), and it is dirty now.
        assert_eq!(pa.free(b, 3), FreeResult::Retained);
        let (d, z) = pa.alloc(2).unwrap();
        assert_eq!(d, b);
        assert!(z);
        assert_eq!(pa.used_pages(), 4);
        // Pages 5..15 of region 0 are free: a 10-page run fits there.
        let (e, _) = pa.alloc(10).unwrap();
        assert_eq!(e, region(0) + 5 * 8192usize);
        // The next one spills to region 1.
        let (f, _) = pa.alloc(10).unwrap();
        assert_eq!(f, region(1));
    }

    #[test]
    fn release_empty_regions() {
        let pa = PageAllocator::new();
        pa.add_region(region(0), false);
        pa.add_region(region(1), true);
        let (a, z) = pa.alloc(10).unwrap();
        assert!(z, "region 0 was not zeroed");
        let (b, _) = pa.alloc(6).unwrap();
        assert_eq!(b, a + 10 * 8192usize);
        let (c, z) = pa.alloc(10).unwrap();
        assert_eq!(c, region(1));
        assert!(!z);
        assert_eq!(pa.free(c, 10), FreeResult::RegionEmpty(region(1)));
        assert_eq!(pa.regions(), vec![region(0)]);
        assert_eq!(pa.free(a, 10), FreeResult::Retained);
        assert_eq!(pa.free(b, 6), FreeResult::RegionEmpty(region(0)));
        assert!(pa.regions().is_empty());
    }
}
