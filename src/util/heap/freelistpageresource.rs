use std::sync::{Mutex, MutexGuard};
use std::sync::atomic::AtomicUsize;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use libc::{c_void, memset};

use util::address::Address;
use util::heap::pageresource::CommonPageResource;
use util::alloc::embedded_meta_data::*;
use util::generic_freelist;
use util::generic_freelist::GenericFreeList;
// #[cfg(target_pointer_width = "32")]
// FIXME: Use `RawMemoryFreeList` for 64-bit machines
use util::int_array_freelist::IntArrayFreeList as FreeList;
use util::heap::layout::vm_layout_constants::*;
use util::heap::layout::heap_layout;
use util::conversions;
use util::constants::*;
use policy::space::Space;
use vm::{VMMemory, Memory};
use super::vmrequest::HEAP_LAYOUT_64BIT;
use super::layout::Mmapper;
use super::layout::heap_layout::MMAPPER;
use super::PageResource;


const SPACE_ALIGN: usize = 1 << 19;


#[derive(Debug)]
pub struct CommonFreeListPageResource {
    free_list: FreeList,
    start: Address,
}

impl CommonFreeListPageResource {
    pub fn resize_freelist(&mut self, start_address: Address) {
        // debug_assert!((HEAP_LAYOUT_64BIT || !contiguous) && !Plan.isInitialized());
        self.start = conversions::align_up(start_address, LOG_BYTES_IN_REGION);
        // self.free_list.resize_freelist();
    }
}

#[derive(Debug)]
pub struct FreeListPageResource<S: Space<PR = FreeListPageResource<S>>> {
    common: CommonPageResource<FreeListPageResource<S>>,
    common_flpr: Box<CommonFreeListPageResource>,
    /** Number of pages to reserve at the start of every allocation */
    meta_data_pages_per_region: usize,
    sync: Mutex<FreeListPageResourceSync>,
}

#[derive(Debug)]
struct FreeListPageResourceSync {
    pages_currently_on_freelist: usize,
    highwater_mark: i32,
}

impl <S: Space<PR = FreeListPageResource<S>>> Deref for FreeListPageResource<S> {
    type Target = CommonFreeListPageResource;

    fn deref(&self) -> &CommonFreeListPageResource {
        &self.common_flpr
    }
}

impl <S: Space<PR = FreeListPageResource<S>>> DerefMut for FreeListPageResource<S> {
    fn deref_mut(&mut self) -> &mut CommonFreeListPageResource {
        &mut self.common_flpr
    }
}


impl<S: Space<PR = FreeListPageResource<S>>> PageResource for FreeListPageResource<S> {
    type Space = S;

    fn meta_data_pages_per_region(&self) -> usize {
        self.meta_data_pages_per_region
    }

    fn common(&self) -> &CommonPageResource<Self> {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource<Self> {
        &mut self.common
    }

    #[allow(mutable_transmutes)]
    fn alloc_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool, tls: *mut c_void) -> Address {
        debug_assert!(self.meta_data_pages_per_region == 0 || required_pages <= PAGES_IN_CHUNK - self.meta_data_pages_per_region);
        let self_mut: &mut Self = unsafe { mem::transmute(self) };
        let mut sync = self.sync.lock().unwrap();
        let mut new_chunk = false;
        let mut page_offset = self_mut.free_list.alloc(required_pages as _);
        if page_offset == generic_freelist::FAILURE && self.common.growable {
            page_offset = self_mut.allocate_contiguous_chunks(required_pages, &mut sync);
            new_chunk = true;
        }
        if page_offset == generic_freelist::FAILURE {
            return unsafe { Address::zero() };
        } else {
            sync.pages_currently_on_freelist -= required_pages;
            if page_offset > sync.highwater_mark {
                if sync.highwater_mark == 0 || (page_offset ^ sync.highwater_mark) > PAGES_IN_REGION as i32 {
                    let regions = 1 + ((page_offset - sync.highwater_mark) >> LOG_PAGES_IN_REGION);
                    let metapages = regions as usize * self.meta_data_pages_per_region;
                    self.common.reserved.fetch_add(metapages, Ordering::Relaxed);
                    self.common.committed.fetch_add(metapages, Ordering::Relaxed);
                    new_chunk = true;
                }
                sync.highwater_mark = page_offset;
            }
        }
        let rtn = self.start + conversions::pages_to_bytes(page_offset as _);
        let bytes = conversions::pages_to_bytes(required_pages);
        // The meta-data portion of reserved Pages was committed above.
        self.commit_pages(reserved_pages, required_pages, tls);
        self.common().space.unwrap().grow_space(rtn, bytes, new_chunk);
        MMAPPER.ensure_mapped(rtn, required_pages);
        if zeroed {
            VMMemory::zero(rtn, bytes);
        }
        rtn
    }

    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages
    }

    fn get_available_physical_pages(&self) -> usize {
        let mut rtn = { self.sync.lock().unwrap().pages_currently_on_freelist };
        if !self.common().contiguous {
            let available_discontiguous_chunks = heap_layout::VM_MAP.get_available_discontiguous_chunks();
            let chunk_consumer_count = heap_layout::VM_MAP.get_chunk_consumer_count();
            let chunks = if available_discontiguous_chunks >= chunk_consumer_count {
                available_discontiguous_chunks - chunk_consumer_count
            } else {
                0
            };
            rtn += chunks * (PAGES_IN_CHUNK - self.meta_data_pages_per_region);
        } else if self.common().growable && HEAP_LAYOUT_64BIT {
            rtn = PAGES_IN_SPACE64 - self.common().reserved.load(Ordering::Relaxed);
        }
        return rtn;
    }
}

impl<S: Space<PR = FreeListPageResource<S>>> FreeListPageResource<S> {
    pub fn new_contiguous(space: &S, start: Address, bytes: usize, meta_data_pages_per_region: usize) -> Self {
        let pages = conversions::bytes_to_pages(bytes);
        let common_flpr = unsafe {
            let mut common_flpr = Box::new(CommonFreeListPageResource {
                free_list: ::std::mem::uninitialized(),
                start,
            });
            ::std::ptr::write(&mut common_flpr.free_list, heap_layout::VM_MAP.create_parent_freelist(pages, PAGES_IN_REGION as _));
            common_flpr
        };
        let growable = HEAP_LAYOUT_64BIT;
        let mut flpr = FreeListPageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                contiguous: true,
                growable,
                space: None,
            },
            common_flpr,
            meta_data_pages_per_region,
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: if growable { 0 } else { pages },
                highwater_mark: 0,
            }),
        };
        if !flpr.common.growable {
            flpr.reserve_metadata(space.common().extent);
            // reserveMetaData(space.getExtent());
            // unimplemented!()
        }
        flpr
    }


    pub fn new_discontiguous(meta_data_pages_per_region: usize) -> Self {
        let common_flpr = unsafe {
            let mut common_flpr = Box::new(CommonFreeListPageResource {
                free_list: ::std::mem::uninitialized(),
                start: AVAILABLE_START,
            });
            ::std::ptr::write(&mut common_flpr.free_list, heap_layout::VM_MAP.create_freelist(&common_flpr));
            common_flpr
        };
        FreeListPageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                contiguous: false,
                growable: true,
                space: None,
            },
            common_flpr,
            meta_data_pages_per_region,
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: 0,
                highwater_mark: 0,
            }),
        }
    }

    fn allocate_contiguous_chunks(&mut self, pages: usize, sync: &mut MutexGuard<FreeListPageResourceSync>) -> i32 {
        debug_assert!(self.meta_data_pages_per_region == 0 || pages <= PAGES_IN_CHUNK - self.meta_data_pages_per_region);
        let mut rtn = generic_freelist::FAILURE;
        let required_chunks = ::policy::space::required_chunks(pages);
        let region = unsafe {
            self.common.space.unwrap().grow_discontiguous_space(required_chunks)
        };

        if !region.is_zero() {
            debug_assert!(region >= self.start);
            let region_start = conversions::bytes_to_pages(region - self.start);
            let region_end = region_start + (required_chunks * PAGES_IN_CHUNK) - 1;
            self.free_list.set_uncoalescable(region_start as _);
            self.free_list.set_uncoalescable(region_end as i32 + 1);
            for p in (region_start..region_end).step_by(PAGES_IN_CHUNK) {
                let mut liberated;
                if p != region_start {
                    self.free_list.clear_uncoalescable(p as _);
                }
                liberated = self.free_list.free(p as _, true); // add chunk to our free list
                debug_assert!(liberated as usize == PAGES_IN_CHUNK + (p - region_start));
                if self.meta_data_pages_per_region > 1 {
                    let meta_data_pages_per_region = self.meta_data_pages_per_region;
                    self.free_list.alloc_from_unit(meta_data_pages_per_region as _, p as _); // carve out space for metadata
                }
                {
                    sync.pages_currently_on_freelist += PAGES_IN_CHUNK - self.meta_data_pages_per_region;
                }
            }
            rtn = self.free_list.alloc(pages as _); // re-do the request which triggered this call
        }
        rtn
    }

    #[allow(mutable_transmutes)]
    fn free_contiguous_chunk(&mut self, chunk: Address) {
        let num_chunks = heap_layout::VM_MAP.get_contiguous_region_chunks(chunk);
        debug_assert!(num_chunks == 1 || self.meta_data_pages_per_region == 0);
        /* nail down all pages associated with the chunk, so it is no longer on our free list */
        let mut chunk_start = conversions::bytes_to_pages(chunk - self.start);
        let chunk_end = chunk_start + (num_chunks * PAGES_IN_CHUNK);
        while chunk_start < chunk_end {
            self.free_list.set_uncoalescable(chunk_start as _);
            if self.meta_data_pages_per_region > 0 {
                self.free_list.free(chunk_start as _, false);  // first free any metadata pages
            }
            let tmp = self.free_list.alloc_from_unit(PAGES_IN_CHUNK as _, chunk_start as _) as usize; // then alloc the entire chunk
            debug_assert!(tmp == chunk_start);
            chunk_start += PAGES_IN_CHUNK;
            {
                let mut sync = self.sync.lock().unwrap();
                sync.pages_currently_on_freelist -= PAGES_IN_CHUNK - self.meta_data_pages_per_region;
            }
        }
        /* now return the address space associated with the chunk for global reuse */
        let space: &mut S = unsafe { mem::transmute(self.common.space.unwrap()) };
        space.release_discontiguous_chunks(chunk);
    }

    fn reserve_metadata(&mut self, extent: usize) {
        let _highwater_mark = 0;
        if self.meta_data_pages_per_region > 0 {
            debug_assert!(((self.start.0 >> LOG_BYTES_IN_REGION) << LOG_BYTES_IN_REGION) == self.start.0);
            let size = (extent >> LOG_BYTES_IN_REGION) << LOG_BYTES_IN_REGION;
            let mut cursor = self.start + size;
            while cursor > self.start {
                cursor = cursor - BYTES_IN_REGION;
                let unit = (cursor - self.start) >> LOG_BYTES_IN_PAGE;
                let meta_data_pages_per_region = self.meta_data_pages_per_region;
                let tmp = self.free_list.alloc_from_unit(meta_data_pages_per_region as _, unit as _) as usize;
                {
                    let mut sync = self.sync.lock().unwrap();
                    sync.pages_currently_on_freelist -= self.meta_data_pages_per_region;
                }
                debug_assert!(tmp == unit);
            }
        }
    }

    pub fn release_pages(&mut self, first: Address) {
        debug_assert!(conversions::is_page_aligned(first));
        let page_offset = conversions::bytes_to_pages(first - self.start);
        let pages = self.free_list.size(page_offset as _);
        // if (VM.config.ZERO_PAGES_ON_RELEASE)
        //     VM.memory.zero(false, first, Conversions.pagesToBytes(pages));
        debug_assert!(pages as usize <= self.common.committed.load(Ordering::Relaxed));
        let me = unsafe { &mut *(self as *mut Self) };
        let freed = {
            let mut sync = self.sync.lock().unwrap();
            self.common.reserved.fetch_sub(pages as _, Ordering::Relaxed);
            self.common.committed.fetch_sub(pages as _, Ordering::Relaxed);
            let freed = me.free_list.free(page_offset as _, true);
            sync.pages_currently_on_freelist += pages as usize;
            freed
        };
        if !self.common.contiguous { // only discontiguous spaces use chunks
            me.release_free_chunks(first, freed as _);
        }
    }


    fn release_free_chunks(&mut self, freed_page: Address, pages_freed: usize) {
        let page_offset = conversions::bytes_to_pages(freed_page - self.start);
        
        if self.meta_data_pages_per_region > 0 {       // can only be a single chunk
            if pages_freed == (PAGES_IN_CHUNK - self.meta_data_pages_per_region) {
                self.free_contiguous_chunk(conversions::chunk_align(freed_page, true));
            }
        } else {                                // may be multiple chunks
            if pages_freed % PAGES_IN_CHUNK == 0 {    // necessary, but not sufficient condition
                /* grow a region of chunks, starting with the chunk containing the freed page */
                let mut region_start = page_offset & !(PAGES_IN_CHUNK - 1);
                let mut next_region_start = region_start + PAGES_IN_CHUNK;
                /* now try to grow (end point pages are marked as non-coalescing) */
                while region_start >= 0 && self.free_list.is_coalescable(region_start as _) {
                    region_start -= PAGES_IN_CHUNK;
                }
                while next_region_start < generic_freelist::MAX_UNITS as usize && self.free_list.is_coalescable(next_region_start as _) {
                    next_region_start += PAGES_IN_CHUNK;
                }
                debug_assert!(region_start >= 0 && next_region_start < generic_freelist::MAX_UNITS as usize);
                if pages_freed == next_region_start - region_start {
                    let start = self.start;
                    self.free_contiguous_chunk(start + conversions::pages_to_bytes(region_start));
                }
            }
        }
    }

}