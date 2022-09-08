use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

use super::layout::map::Map;
use super::layout::vm_layout_constants::{PAGES_IN_CHUNK, PAGES_IN_SPACE64};
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::PageResource;
use crate::util::address::Address;
use crate::util::alloc::embedded_meta_data::*;
use crate::util::constants::*;
use crate::util::conversions;
use crate::util::generic_freelist;
use crate::util::generic_freelist::GenericFreeList;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::memory;
use crate::util::opaque_pointer::*;
use crate::vm::*;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

const UNINITIALIZED_WATER_MARK: i32 = -1;

pub struct CommonFreeListPageResource {
    free_list: Box<<VMMap as Map>::FreeList>,
    start: Address,
}

impl CommonFreeListPageResource {
    pub fn get_start(&self) -> Address {
        self.start
    }

    pub fn resize_freelist(&mut self, start_address: Address) {
        self.start = start_address.align_up(BYTES_IN_REGION);
    }
}

pub struct FreeListPageResource<VM: VMBinding> {
    common: CommonPageResource,
    common_flpr: Box<CommonFreeListPageResource>,
    /** Number of pages to reserve at the start of every allocation */
    meta_data_pages_per_region: usize,
    sync: Mutex<FreeListPageResourceSync>,
    _p: PhantomData<VM>,
    /// Protect memory on release, and unprotect on re-allocate.
    pub(crate) protect_memory_on_release: bool,
}

struct FreeListPageResourceSync {
    pages_currently_on_freelist: usize,
    highwater_mark: i32,
}

impl<VM: VMBinding> Deref for FreeListPageResource<VM> {
    type Target = CommonFreeListPageResource;

    fn deref(&self) -> &CommonFreeListPageResource {
        &self.common_flpr
    }
}

impl<VM: VMBinding> DerefMut for FreeListPageResource<VM> {
    fn deref_mut(&mut self) -> &mut CommonFreeListPageResource {
        &mut self.common_flpr
    }
}

impl<VM: VMBinding> PageResource<VM> for FreeListPageResource<VM> {
    fn common(&self) -> &CommonPageResource {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource {
        &mut self.common
    }

    fn get_available_physical_pages(&self) -> usize {
        let mut rtn = self.sync.lock().unwrap().pages_currently_on_freelist;
        if !self.common.contiguous {
            let chunks: usize = self
                .common
                .vm_map
                .get_available_discontiguous_chunks()
                .saturating_sub(self.common.vm_map.get_chunk_consumer_count());
            rtn += chunks * (PAGES_IN_CHUNK - self.meta_data_pages_per_region);
        } else if self.common.growable && cfg!(target_pointer_width = "64") {
            rtn = PAGES_IN_SPACE64 - self.reserved_pages();
        }

        rtn
    }

    fn alloc_pages(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        debug_assert!(
            self.meta_data_pages_per_region == 0
                || required_pages <= PAGES_IN_CHUNK - self.meta_data_pages_per_region
        );
        // FIXME: We need a safe implementation
        #[allow(clippy::cast_ref_to_mut)]
        let self_mut: &mut Self = unsafe { &mut *(self as *const _ as *mut _) };
        let mut sync = self.sync.lock().unwrap();
        let mut new_chunk = false;
        let mut page_offset = self_mut.free_list.alloc(required_pages as _);
        if page_offset == generic_freelist::FAILURE && self.common.growable {
            page_offset =
                self_mut.allocate_contiguous_chunks(space_descriptor, required_pages, &mut sync);
            new_chunk = true;
        }

        if page_offset == generic_freelist::FAILURE {
            return Result::Err(PRAllocFail);
        } else {
            sync.pages_currently_on_freelist -= required_pages;
            if page_offset > sync.highwater_mark {
                if sync.highwater_mark == UNINITIALIZED_WATER_MARK
                    || (page_offset ^ sync.highwater_mark) > PAGES_IN_REGION as i32
                {
                    let regions = 1 + ((page_offset - sync.highwater_mark) >> LOG_PAGES_IN_REGION);
                    let metapages = regions as usize * self.meta_data_pages_per_region;
                    self.common.accounting.reserve_and_commit(metapages);
                    new_chunk = true;
                }
                sync.highwater_mark = page_offset;
            }
        }

        let rtn = self.start + conversions::pages_to_bytes(page_offset as _);
        // The meta-data portion of reserved Pages was committed above.
        self.commit_pages(reserved_pages, required_pages, tls);
        if self.protect_memory_on_release && !new_chunk {
            use crate::util::heap::layout::Mmapper;
            use crate::MMAPPER;
            // This check is necessary to prevent us from mprotecting an address that is not yet mapped by mmapper.
            // See https://github.com/mmtk/mmtk-core/issues/400.
            // It is possible that one thread gets a new chunk, and returns from this function. However, the Space.acquire()
            // has not yet call ensure_mapped() for it. So the chunk is not yet mmapped. At this point, if another thread calls
            // this function, and get a few more pages from the same chunk, it is no longer seen as 'new_chunk', and we
            // will try to munprotect on it. But the chunk may not yet be mapped.
            //
            // If we want to improve and get rid of this loop, we need to move this munprotect to anywhere after the ensure_mapped() call
            // in Space.acquire(). We can either move it the option of 'protect_on_release' to space, or have a call to page resource
            // after ensure_mapped(). However, I think this is sufficient given that this option is only used for PageProtect for debugging use.
            while !MMAPPER.is_mapped_address(rtn) {}
            self.munprotect(rtn, self.free_list.size(page_offset as _) as _)
        };
        Result::Ok(PRAllocResult {
            start: rtn,
            pages: required_pages,
            new_chunk,
        })
    }

    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages
    }
}

impl<VM: VMBinding> FreeListPageResource<VM> {
    pub fn new_contiguous(
        start: Address,
        bytes: usize,
        meta_data_pages_per_region: usize,
        vm_map: &'static VMMap,
    ) -> Self {
        let pages = conversions::bytes_to_pages(bytes);
        // We use MaybeUninit::uninit().assume_init(), which is nul, for a Box value, which cannot be null.
        // FIXME: We should try either remove this kind of circular dependency or use MaybeUninit<T> instead of Box<T>
        #[allow(invalid_value)]
        #[allow(clippy::uninit_assumed_init)]
        let common_flpr = unsafe {
            let mut common_flpr = Box::new(CommonFreeListPageResource {
                free_list: MaybeUninit::uninit().assume_init(),
                start,
            });
            ::std::ptr::write(
                &mut common_flpr.free_list,
                vm_map.create_parent_freelist(&common_flpr, pages, PAGES_IN_REGION as _),
            );
            common_flpr
        };
        let growable = cfg!(target_pointer_width = "64");
        let mut flpr = FreeListPageResource {
            common: CommonPageResource::new(true, growable, vm_map),
            common_flpr,
            meta_data_pages_per_region,
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: if growable { 0 } else { pages },
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: false,
        };
        if !flpr.common.growable {
            // For non-growable space, we just need to reserve metadata according to the requested size.
            flpr.reserve_metadata(bytes);
            // reserveMetaData(space.getExtent());
            // unimplemented!()
        }
        flpr
    }

    pub fn new_discontiguous(meta_data_pages_per_region: usize, vm_map: &'static VMMap) -> Self {
        // We use MaybeUninit::uninit().assume_init(), which is nul, for a Box value, which cannot be null.
        // FIXME: We should try either remove this kind of circular dependency or use MaybeUninit<T> instead of Box<T>
        #[allow(invalid_value)]
        #[allow(clippy::uninit_assumed_init)]
        let common_flpr = unsafe {
            let mut common_flpr = Box::new(CommonFreeListPageResource {
                free_list: MaybeUninit::uninit().assume_init(),
                start: AVAILABLE_START,
            });
            ::std::ptr::write(
                &mut common_flpr.free_list,
                vm_map.create_freelist(&common_flpr),
            );
            common_flpr
        };
        FreeListPageResource {
            common: CommonPageResource::new(false, true, vm_map),
            common_flpr,
            meta_data_pages_per_region,
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: 0,
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: false,
        }
    }

    /// Protect the memory
    fn mprotect(&self, start: Address, pages: usize) {
        // We may fail here for ENOMEM, especially in PageProtect plan.
        // See: https://man7.org/linux/man-pages/man2/mprotect.2.html#ERRORS
        // > Changing the protection of a memory region would result in
        // > the total number of mappings with distinct attributes
        // > (e.g., read versus read/write protection) exceeding the
        // > allowed maximum.
        assert!(self.protect_memory_on_release);
        // We are not using mmapper.protect(). mmapper.protect() protects the whole chunk and
        // may protect memory that is still in use.
        if let Err(e) = memory::mprotect(start, conversions::pages_to_bytes(pages)) {
            panic!(
                "Failed at protecting memory (starting at {}): {:?}",
                start, e
            );
        }
    }

    /// Unprotect the memory
    fn munprotect(&self, start: Address, pages: usize) {
        assert!(self.protect_memory_on_release);
        if let Err(e) = memory::munprotect(start, conversions::pages_to_bytes(pages)) {
            panic!(
                "Failed at unprotecting memory (starting at {}): {:?}",
                start, e
            );
        }
    }

    fn allocate_contiguous_chunks(
        &mut self,
        space_descriptor: SpaceDescriptor,
        pages: usize,
        sync: &mut MutexGuard<FreeListPageResourceSync>,
    ) -> i32 {
        debug_assert!(
            self.meta_data_pages_per_region == 0
                || pages <= PAGES_IN_CHUNK - self.meta_data_pages_per_region
        );
        let mut rtn = generic_freelist::FAILURE;
        let required_chunks = crate::policy::space::required_chunks(pages);
        let region = self
            .common
            .grow_discontiguous_space(space_descriptor, required_chunks);

        if !region.is_zero() {
            let region_start = conversions::bytes_to_pages(region - self.start);
            let region_end = region_start + (required_chunks * PAGES_IN_CHUNK) - 1;
            self.free_list.set_uncoalescable(region_start as _);
            self.free_list.set_uncoalescable(region_end as i32 + 1);
            for p in (region_start..region_end).step_by(PAGES_IN_CHUNK) {
                if p != region_start {
                    self.free_list.clear_uncoalescable(p as _);
                }
                let liberated = self.free_list.free(p as _, true); // add chunk to our free list
                debug_assert!(liberated as usize == PAGES_IN_CHUNK + (p - region_start));
                if self.meta_data_pages_per_region > 1 {
                    let meta_data_pages_per_region = self.meta_data_pages_per_region;
                    self.free_list
                        .alloc_from_unit(meta_data_pages_per_region as _, p as _);
                    // carve out space for metadata
                }
                {
                    sync.pages_currently_on_freelist +=
                        PAGES_IN_CHUNK - self.meta_data_pages_per_region;
                }
            }
            rtn = self.free_list.alloc(pages as _); // re-do the request which triggered this call
        }
        rtn
    }

    fn free_contiguous_chunk(&mut self, chunk: Address) {
        let num_chunks = self.vm_map().get_contiguous_region_chunks(chunk);
        debug_assert!(num_chunks == 1 || self.meta_data_pages_per_region == 0);
        /* nail down all pages associated with the chunk, so it is no longer on our free list */
        let mut chunk_start = conversions::bytes_to_pages(chunk - self.start);
        let chunk_end = chunk_start + (num_chunks * PAGES_IN_CHUNK);
        while chunk_start < chunk_end {
            self.free_list.set_uncoalescable(chunk_start as _);
            if self.meta_data_pages_per_region > 0 {
                self.free_list.free(chunk_start as _, false); // first free any metadata pages
            }
            let tmp = self
                .free_list
                .alloc_from_unit(PAGES_IN_CHUNK as _, chunk_start as _)
                as usize; // then alloc the entire chunk
            debug_assert!(tmp == chunk_start);
            chunk_start += PAGES_IN_CHUNK;
            {
                let mut sync = self.sync.lock().unwrap();
                sync.pages_currently_on_freelist -=
                    PAGES_IN_CHUNK - self.meta_data_pages_per_region;
            }
        }
        /* now return the address space associated with the chunk for global reuse */
        self.common.release_discontiguous_chunks(chunk);
    }

    fn reserve_metadata(&mut self, extent: usize) {
        if self.meta_data_pages_per_region > 0 {
            debug_assert!(self.start.is_aligned_to(BYTES_IN_REGION));
            let size = (extent >> LOG_BYTES_IN_REGION) << LOG_BYTES_IN_REGION;
            let mut cursor = self.start + size;
            while cursor > self.start {
                cursor -= BYTES_IN_REGION;
                let unit = (cursor - self.start) >> LOG_BYTES_IN_PAGE;
                let meta_data_pages_per_region = self.meta_data_pages_per_region;
                let tmp = self
                    .free_list
                    .alloc_from_unit(meta_data_pages_per_region as _, unit as _)
                    as usize;
                {
                    let mut sync = self.sync.lock().unwrap();
                    sync.pages_currently_on_freelist -= self.meta_data_pages_per_region;
                }
                debug_assert!(tmp == unit);
            }
        }
    }

    pub fn release_pages(&self, first: Address) {
        debug_assert!(conversions::is_page_aligned(first));
        let page_offset = conversions::bytes_to_pages(first - self.start);
        let pages = self.free_list.size(page_offset as _);
        // if (VM.config.ZERO_PAGES_ON_RELEASE)
        //     VM.memory.zero(false, first, Conversions.pagesToBytes(pages));
        debug_assert!(pages as usize <= self.common.accounting.get_committed_pages());

        if self.protect_memory_on_release {
            self.mprotect(first, pages as _);
        }

        // FIXME
        #[allow(clippy::cast_ref_to_mut)]
        let me = unsafe { &mut *(self as *const _ as *mut Self) };
        let freed = {
            let mut sync = self.sync.lock().unwrap();
            self.common.accounting.release(pages as _);
            let freed = me.free_list.free(page_offset as _, true);
            sync.pages_currently_on_freelist += pages as usize;
            freed
        };
        if !self.common.contiguous {
            // only discontiguous spaces use chunks
            me.release_free_chunks(first, freed as _);
        }
    }

    fn release_free_chunks(&mut self, freed_page: Address, pages_freed: usize) {
        let page_offset = conversions::bytes_to_pages(freed_page - self.start);

        if self.meta_data_pages_per_region > 0 {
            // can only be a single chunk
            if pages_freed == (PAGES_IN_CHUNK - self.meta_data_pages_per_region) {
                self.free_contiguous_chunk(conversions::chunk_align_down(freed_page));
            }
        } else {
            // may be multiple chunks
            if pages_freed % PAGES_IN_CHUNK == 0 {
                // necessary, but not sufficient condition
                /* grow a region of chunks, starting with the chunk containing the freed page */
                let mut region_start = page_offset & !(PAGES_IN_CHUNK - 1);
                let mut next_region_start = region_start + PAGES_IN_CHUNK;
                /* now try to grow (end point pages are marked as non-coalescing) */
                while self.free_list.is_coalescable(region_start as _) {
                    // region_start is guaranteed to be positive. Otherwise this line will fail due to subtraction overflow.
                    region_start -= PAGES_IN_CHUNK;
                }
                while next_region_start < generic_freelist::MAX_UNITS as usize
                    && self.free_list.is_coalescable(next_region_start as _)
                {
                    next_region_start += PAGES_IN_CHUNK;
                }
                debug_assert!(next_region_start < generic_freelist::MAX_UNITS as usize);
                if pages_freed == next_region_start - region_start {
                    let start = self.start;
                    self.free_contiguous_chunk(start + conversions::pages_to_bytes(region_start));
                }
            }
        }
    }
}
