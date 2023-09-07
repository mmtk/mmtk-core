use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

use super::layout::vm_layout::PAGES_IN_CHUNK;
use super::layout::VMMap;
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::PageResource;
use crate::mmtk::MMAPPER;
use crate::util::address::Address;
use crate::util::alloc::embedded_meta_data::*;
use crate::util::conversions;
use crate::util::freelist;
use crate::util::freelist::FreeList;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::memory;
use crate::util::opaque_pointer::*;
use crate::vm::*;
use std::marker::PhantomData;

const UNINITIALIZED_WATER_MARK: i32 = -1;

pub struct CommonFreeListPageResource {
    pub(crate) free_list: Box<dyn FreeList>,
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
    inner: UnsafeCell<FreeListPageResourceInner>,
    sync: Mutex<FreeListPageResourceSync>,
    _p: PhantomData<VM>,
    /// Protect memory on release, and unprotect on re-allocate.
    pub(crate) protect_memory_on_release: bool,
}

unsafe impl<VM: VMBinding> Send for FreeListPageResource<VM> {}
unsafe impl<VM: VMBinding> Sync for FreeListPageResource<VM> {}

struct FreeListPageResourceInner {
    common: CommonPageResource,
    common_flpr: Box<CommonFreeListPageResource>,
}

struct FreeListPageResourceSync {
    pages_currently_on_freelist: usize,
    highwater_mark: i32,
}

impl<VM: VMBinding> Deref for FreeListPageResource<VM> {
    type Target = CommonFreeListPageResource;

    fn deref(&self) -> &CommonFreeListPageResource {
        &self.inner().common_flpr
    }
}

impl<VM: VMBinding> DerefMut for FreeListPageResource<VM> {
    fn deref_mut(&mut self) -> &mut CommonFreeListPageResource {
        &mut self.inner.get_mut().common_flpr
    }
}

impl Deref for FreeListPageResourceInner {
    type Target = CommonFreeListPageResource;

    fn deref(&self) -> &CommonFreeListPageResource {
        &self.common_flpr
    }
}

impl DerefMut for FreeListPageResourceInner {
    fn deref_mut(&mut self) -> &mut CommonFreeListPageResource {
        &mut self.common_flpr
    }
}

impl<VM: VMBinding> PageResource<VM> for FreeListPageResource<VM> {
    fn common(&self) -> &CommonPageResource {
        &self.inner().common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource {
        &mut self.inner.get_mut().common
    }

    fn get_available_physical_pages(&self) -> usize {
        let mut rtn = self.sync.lock().unwrap().pages_currently_on_freelist;
        if !self.inner().common.contiguous {
            let chunks: usize = self
                .inner()
                .common
                .vm_map
                .get_available_discontiguous_chunks()
                .saturating_sub(self.inner().common.vm_map.get_chunk_consumer_count());
            rtn += chunks * PAGES_IN_CHUNK;
        } else if self.inner().common.growable && cfg!(target_pointer_width = "64") {
            rtn = vm_layout().pages_in_space64() - self.reserved_pages();
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
        // FIXME: We need a safe implementation
        let self_mut = unsafe { self.inner_mut() };
        let mut sync = self.sync.lock().unwrap();
        let mut new_chunk = false;
        let mut page_offset = self_mut.free_list.alloc(required_pages as _);
        if page_offset == freelist::FAILURE && self.inner().common.growable {
            page_offset = unsafe {
                self.allocate_contiguous_chunks(space_descriptor, required_pages, &mut sync)
            };
            new_chunk = true;
        }

        if page_offset == freelist::FAILURE {
            return Result::Err(PRAllocFail);
        } else {
            sync.pages_currently_on_freelist -= required_pages;
            if page_offset > sync.highwater_mark {
                if sync.highwater_mark == UNINITIALIZED_WATER_MARK
                    || (page_offset ^ sync.highwater_mark) > PAGES_IN_REGION as i32
                {
                    new_chunk = true;
                }
                sync.highwater_mark = page_offset;
            }
        }

        let rtn = self.start + conversions::pages_to_bytes(page_offset as _);
        // The meta-data portion of reserved Pages was committed above.
        self.commit_pages(reserved_pages, required_pages, tls);
        if self.protect_memory_on_release {
            if !new_chunk {
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
                while !new_chunk && !MMAPPER.is_mapped_address(rtn) {}
                self.munprotect(rtn, self.free_list.size(page_offset as _) as _)
            } else if !self.common().contiguous && new_chunk {
                // Don't unprotect if this is a new unmapped discontiguous chunk
                // For a new mapped discontiguous chunk, this should previously be released and protected by us.
                // We still need to unprotect it.
                if MMAPPER.is_mapped_address(rtn) {
                    self.munprotect(rtn, self.free_list.size(page_offset as _) as _)
                }
            }
        };
        Result::Ok(PRAllocResult {
            start: rtn,
            pages: required_pages,
            new_chunk,
        })
    }
}

impl<VM: VMBinding> FreeListPageResource<VM> {
    pub fn new_contiguous(start: Address, bytes: usize, vm_map: &'static dyn VMMap) -> Self {
        let pages = conversions::bytes_to_pages(bytes);
        let common_flpr = {
            let common_flpr = Box::new(CommonFreeListPageResource {
                free_list: vm_map.create_parent_freelist(start, pages, PAGES_IN_REGION as _),
                start,
            });
            // `CommonFreeListPageResource` lives as a member in space instances.
            // Since `Space` instances are always stored as global variables, so it is okay here
            // to turn `&CommonFreeListPageResource` into `&'static CommonFreeListPageResource`
            unsafe {
                vm_map.bind_freelist(&*(&common_flpr as &CommonFreeListPageResource as *const _));
            }
            common_flpr
        };
        let growable = cfg!(target_pointer_width = "64");
        FreeListPageResource {
            inner: UnsafeCell::new(FreeListPageResourceInner {
                common: CommonPageResource::new(true, growable, vm_map),
                common_flpr,
            }),
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: if growable { 0 } else { pages },
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: false,
        }
    }

    pub fn new_discontiguous(vm_map: &'static dyn VMMap) -> Self {
        let common_flpr = {
            let start = vm_layout().available_start();
            let common_flpr = Box::new(CommonFreeListPageResource {
                free_list: vm_map.create_freelist(start),
                start,
            });
            // `CommonFreeListPageResource` lives as a member in space instances.
            // Since `Space` instances are always stored as global variables, so it is okay here
            // to turn `&CommonFreeListPageResource` into `&'static CommonFreeListPageResource`
            unsafe {
                vm_map.bind_freelist(&*(&common_flpr as &CommonFreeListPageResource as *const _));
            }
            common_flpr
        };
        FreeListPageResource {
            inner: UnsafeCell::new(FreeListPageResourceInner {
                common: CommonPageResource::new(false, true, vm_map),
                common_flpr,
            }),
            sync: Mutex::new(FreeListPageResourceSync {
                pages_currently_on_freelist: 0,
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: false,
        }
    }

    fn inner(&self) -> &FreeListPageResourceInner {
        unsafe { &*self.inner.get() }
    }
    #[allow(clippy::mut_from_ref)]
    unsafe fn inner_mut(&self) -> &mut FreeListPageResourceInner {
        &mut *self.inner.get()
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

    pub(crate) fn allocate_one_chunk_no_commit(
        &self,
        space_descriptor: SpaceDescriptor,
    ) -> Result<PRAllocResult, PRAllocFail> {
        assert!(self.inner().common.growable);
        // FIXME: We need a safe implementation
        let mut sync = self.sync.lock().unwrap();
        let page_offset =
            unsafe { self.allocate_contiguous_chunks(space_descriptor, PAGES_IN_CHUNK, &mut sync) };

        if page_offset == freelist::FAILURE {
            return Result::Err(PRAllocFail);
        } else {
            sync.pages_currently_on_freelist -= PAGES_IN_CHUNK;
            if page_offset > sync.highwater_mark {
                sync.highwater_mark = page_offset;
            }
        }

        let rtn = self.start + conversions::pages_to_bytes(page_offset as _);
        Result::Ok(PRAllocResult {
            start: rtn,
            pages: PAGES_IN_CHUNK,
            new_chunk: true,
        })
    }

    unsafe fn allocate_contiguous_chunks(
        &self,
        space_descriptor: SpaceDescriptor,
        pages: usize,
        sync: &mut MutexGuard<FreeListPageResourceSync>,
    ) -> i32 {
        let mut rtn = freelist::FAILURE;
        let required_chunks = crate::policy::space::required_chunks(pages);
        let region = self
            .inner()
            .common
            .grow_discontiguous_space(space_descriptor, required_chunks);

        if !region.is_zero() {
            let region_start = conversions::bytes_to_pages(region - self.start);
            let region_end = region_start + (required_chunks * PAGES_IN_CHUNK) - 1;
            self.inner_mut()
                .free_list
                .set_uncoalescable(region_start as _);
            self.inner_mut()
                .free_list
                .set_uncoalescable(region_end as i32 + 1);
            for p in (region_start..region_end).step_by(PAGES_IN_CHUNK) {
                if p != region_start {
                    self.inner_mut().free_list.clear_uncoalescable(p as _);
                }
                let liberated = self.inner_mut().free_list.free(p as _, true); // add chunk to our free list
                debug_assert!(liberated as usize == PAGES_IN_CHUNK + (p - region_start));
                sync.pages_currently_on_freelist += PAGES_IN_CHUNK;
            }
            rtn = self.inner_mut().free_list.alloc(pages as _); // re-do the request which triggered this call
        }

        rtn
    }

    unsafe fn free_contiguous_chunk(&self, chunk: Address, sync: &mut FreeListPageResourceSync) {
        let num_chunks = self.vm_map().get_contiguous_region_chunks(chunk);
        /* nail down all pages associated with the chunk, so it is no longer on our free list */
        let mut chunk_start = conversions::bytes_to_pages(chunk - self.start);
        let chunk_end = chunk_start + (num_chunks * PAGES_IN_CHUNK);
        while chunk_start < chunk_end {
            self.inner_mut()
                .free_list
                .set_uncoalescable(chunk_start as _);
            let tmp = self
                .inner_mut()
                .free_list
                .alloc_from_unit(PAGES_IN_CHUNK as _, chunk_start as _)
                as usize; // then alloc the entire chunk
            debug_assert!(tmp == chunk_start);
            chunk_start += PAGES_IN_CHUNK;
            sync.pages_currently_on_freelist -= PAGES_IN_CHUNK;
        }
        /* now return the address space associated with the chunk for global reuse */

        self.inner_mut().common.release_discontiguous_chunks(chunk);
    }

    pub fn release_pages(&self, first: Address) {
        debug_assert!(conversions::is_page_aligned(first));
        let page_offset = conversions::bytes_to_pages(first - self.start);
        let pages = self.free_list.size(page_offset as _);
        // if (VM.config.ZERO_PAGES_ON_RELEASE)
        //     VM.memory.zero(false, first, Conversions.pagesToBytes(pages));
        debug_assert!(pages as usize <= self.inner().common.accounting.get_committed_pages());

        if self.protect_memory_on_release {
            self.mprotect(first, pages as _);
        }

        let mut sync = self.sync.lock().unwrap();
        // FIXME: We need a safe implementation
        let me = unsafe { self.inner_mut() };
        self.inner().common.accounting.release(pages as _);
        let freed = me.free_list.free(page_offset as _, true);
        sync.pages_currently_on_freelist += pages as usize;
        if !self.inner().common.contiguous {
            // only discontiguous spaces use chunks
            self.release_free_chunks(first, freed as _, &mut sync);
        }
    }

    fn release_free_chunks(
        &self,
        freed_page: Address,
        pages_freed: usize,
        sync: &mut FreeListPageResourceSync,
    ) {
        let page_offset = conversions::bytes_to_pages(freed_page - self.start);

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
            while next_region_start < freelist::MAX_UNITS as usize
                && self.free_list.is_coalescable(next_region_start as _)
            {
                next_region_start += PAGES_IN_CHUNK;
            }
            debug_assert!(next_region_start < freelist::MAX_UNITS as usize);
            if pages_freed == next_region_start - region_start {
                let start = self.start;
                unsafe {
                    self.free_contiguous_chunk(
                        start + conversions::pages_to_bytes(region_start),
                        sync,
                    );
                }
            }
        }
    }
}
