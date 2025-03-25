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
use crate::util::heap::layout::CreateFreeListResult;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::memory;
use crate::util::opaque_pointer::*;
use crate::util::raw_memory_freelist::RawMemoryFreeList;
use crate::vm::*;
use std::marker::PhantomData;

const UNINITIALIZED_WATER_MARK: i32 = -1;

pub struct FreeListPageResource<VM: VMBinding> {
    common: CommonPageResource,
    sync: Mutex<FreeListPageResourceSync>,
    _p: PhantomData<VM>,
    /// Protect memory on release, and unprotect on re-allocate.
    pub(crate) protect_memory_on_release: Option<memory::MmapProtection>,
}

unsafe impl<VM: VMBinding> Send for FreeListPageResource<VM> {}
unsafe impl<VM: VMBinding> Sync for FreeListPageResource<VM> {}

struct FreeListPageResourceSync {
    pub(crate) free_list: Box<dyn FreeList>,
    pages_currently_on_freelist: usize,
    start: Address,
    highwater_mark: i32,
}

impl<VM: VMBinding> PageResource<VM> for FreeListPageResource<VM> {
    fn common(&self) -> &CommonPageResource {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource {
        &mut self.common
    }
    fn update_discontiguous_start(&mut self, start: Address) {
        // Only discontiguous FreeListPageResource needs adjustment.
        if !self.common.contiguous {
            // The adjustment happens when we still have a `&mut MMTK`.
            // We bypass the mutex lock by calling `get_mut`.
            let sync = self.sync.get_mut().unwrap();
            sync.start = start.align_up(BYTES_IN_REGION);
        }
    }

    fn get_available_physical_pages(&self) -> usize {
        let mut rtn = {
            let sync = self.sync.lock().unwrap();
            sync.pages_currently_on_freelist
        };

        if !self.common.contiguous {
            let chunks: usize = self
                .common
                .vm_map
                .get_available_discontiguous_chunks()
                .saturating_sub(self.common.vm_map.get_chunk_consumer_count());
            rtn += chunks * PAGES_IN_CHUNK;
        } else if self.common.growable && cfg!(target_pointer_width = "64") {
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
        let mut sync = self.sync.lock().unwrap();
        let mut new_chunk = false;
        let mut page_offset = sync.free_list.alloc(required_pages as _);
        if page_offset == freelist::FAILURE && self.common.growable {
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

        let rtn = sync.start + conversions::pages_to_bytes(page_offset as _);
        // The meta-data portion of reserved Pages was committed above.
        self.commit_pages(reserved_pages, required_pages, tls);
        if self.protect_memory_on_release.is_some() {
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
                self.munprotect(rtn, sync.free_list.size(page_offset as _) as _)
            } else if !self.common.contiguous && new_chunk {
                // Don't unprotect if this is a new unmapped discontiguous chunk
                // For a new mapped discontiguous chunk, this should previously be released and protected by us.
                // We still need to unprotect it.
                if MMAPPER.is_mapped_address(rtn) {
                    self.munprotect(rtn, sync.free_list.size(page_offset as _) as _)
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
        let pages = conversions::bytes_to_pages_up(bytes);
        let CreateFreeListResult {
            free_list,
            space_displacement,
        } = vm_map.create_parent_freelist(start, pages, PAGES_IN_REGION as _);

        // If it is RawMemoryFreeList, it will occupy `space_displacement` bytes at the start of
        // the space.  We add it to the start address.
        let actual_start = start + space_displacement;
        debug!(
            "  in new_contiguous: space_displacement = {:?}, actual_start = {}",
            space_displacement, actual_start
        );

        let growable = cfg!(target_pointer_width = "64");
        FreeListPageResource {
            common: CommonPageResource::new(true, growable, vm_map),
            sync: Mutex::new(FreeListPageResourceSync {
                free_list,
                pages_currently_on_freelist: if growable { 0 } else { pages },
                start: actual_start,
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: None,
        }
    }

    pub fn new_discontiguous(vm_map: &'static dyn VMMap) -> Self {
        // This is a place-holder value that is used by neither `vm_map.create_freelist` nor the
        // space.  The location of discontiguous spaces is not determined before all contiguous
        // spaces are places, at which time the starting address of discontiguous spaces will be
        // updated to the correct value.
        let start = vm_layout().available_start();

        let CreateFreeListResult {
            free_list,
            space_displacement,
        } = vm_map.create_freelist(start);

        // In theory, nothing prevents us from using `RawMemoryFreeList` for discontiguous spaces.
        // But in the current implementation, only `Map32` supports discontiguous spaces, and
        // `Map32` only uses `IntArrayFreeList`.
        debug_assert!(
            free_list.downcast_ref::<RawMemoryFreeList>().is_none(),
            "We can't allocate RawMemoryFreeList for discontiguous spaces."
        );

        // Discontiguous free list page resources are only used by `Map32` which uses
        // `IntArrayFreeList` exclusively.  It does not have space displacement.
        debug_assert_eq!(space_displacement, 0);
        debug!("new_discontiguous. start: {start})");

        FreeListPageResource {
            common: CommonPageResource::new(false, true, vm_map),
            sync: Mutex::new(FreeListPageResourceSync {
                free_list,
                pages_currently_on_freelist: 0,
                start,
                highwater_mark: UNINITIALIZED_WATER_MARK,
            }),
            _p: PhantomData,
            protect_memory_on_release: None,
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
        assert!(self.protect_memory_on_release.is_some());
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
        assert!(self.protect_memory_on_release.is_some());
        if let Err(e) = memory::munprotect(
            start,
            conversions::pages_to_bytes(pages),
            self.protect_memory_on_release.unwrap(),
        ) {
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
        assert!(self.common.growable);
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

        let rtn = sync.start + conversions::pages_to_bytes(page_offset as _);
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
        let region = self.common.grow_discontiguous_space(
            space_descriptor,
            required_chunks,
            Some(sync.free_list.as_mut()),
        );

        if !region.is_zero() {
            let region_start = conversions::bytes_to_pages_up(region - sync.start);
            let region_end = region_start + (required_chunks * PAGES_IN_CHUNK) - 1;
            sync.free_list.set_uncoalescable(region_start as _);
            sync.free_list.set_uncoalescable(region_end as i32 + 1);
            for p in (region_start..region_end).step_by(PAGES_IN_CHUNK) {
                if p != region_start {
                    sync.free_list.clear_uncoalescable(p as _);
                }
                let liberated = sync.free_list.free(p as _, true); // add chunk to our free list
                debug_assert!(liberated as usize == PAGES_IN_CHUNK + (p - region_start));
                sync.pages_currently_on_freelist += PAGES_IN_CHUNK;
            }
            rtn = sync.free_list.alloc(pages as _); // re-do the request which triggered this call
        }

        rtn
    }

    unsafe fn free_contiguous_chunk(&self, chunk: Address, sync: &mut FreeListPageResourceSync) {
        let num_chunks = self.vm_map().get_contiguous_region_chunks(chunk);
        /* nail down all pages associated with the chunk, so it is no longer on our free list */
        let mut chunk_start = conversions::bytes_to_pages_up(chunk - sync.start);
        let chunk_end = chunk_start + (num_chunks * PAGES_IN_CHUNK);
        while chunk_start < chunk_end {
            sync.free_list.set_uncoalescable(chunk_start as _);
            let tmp = sync
                .free_list
                .alloc_from_unit(PAGES_IN_CHUNK as _, chunk_start as _)
                as usize; // then alloc the entire chunk
            debug_assert!(tmp == chunk_start);
            chunk_start += PAGES_IN_CHUNK;
            sync.pages_currently_on_freelist -= PAGES_IN_CHUNK;
        }
        /* now return the address space associated with the chunk for global reuse */

        self.common.release_discontiguous_chunks(chunk);
    }

    /// Release pages previously allocated by `alloc_pages`.
    ///
    /// Warning: This method acquires the mutex `self.sync`.  If multiple threads release pages
    /// concurrently, the lock contention will become a performance bottleneck.  This is especially
    /// problematic for plans that sweep objects in bulk in the `Release` stage.  Spaces except the
    /// large object space are recommended to use [`BlockPageResource`] whenever possible.
    ///
    /// [`BlockPageResource`]: crate::util::heap::blockpageresource::BlockPageResource
    pub fn release_pages(&self, first: Address) {
        debug_assert!(conversions::is_page_aligned(first));
        let mut sync = self.sync.lock().unwrap();
        let page_offset = conversions::bytes_to_pages_up(first - sync.start);
        let pages = sync.free_list.size(page_offset as _);
        // if (VM.config.ZERO_PAGES_ON_RELEASE)
        //     VM.memory.zero(false, first, Conversions.pagesToBytes(pages));
        debug_assert!(pages as usize <= self.common.accounting.get_committed_pages());

        if self.protect_memory_on_release.is_some() {
            self.mprotect(first, pages as _);
        }

        self.common.accounting.release(pages as _);
        let freed = sync.free_list.free(page_offset as _, true);
        sync.pages_currently_on_freelist += pages as usize;
        if !self.common.contiguous {
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
        let page_offset = conversions::bytes_to_pages_up(freed_page - sync.start);

        // may be multiple chunks
        if pages_freed % PAGES_IN_CHUNK == 0 {
            // necessary, but not sufficient condition
            /* grow a region of chunks, starting with the chunk containing the freed page */
            let mut region_start = page_offset & !(PAGES_IN_CHUNK - 1);
            let mut next_region_start = region_start + PAGES_IN_CHUNK;
            /* now try to grow (end point pages are marked as non-coalescing) */
            while sync.free_list.is_coalescable(region_start as _) {
                // region_start is guaranteed to be positive. Otherwise this line will fail due to subtraction overflow.
                region_start -= PAGES_IN_CHUNK;
            }
            while next_region_start < freelist::MAX_UNITS as usize
                && sync.free_list.is_coalescable(next_region_start as _)
            {
                next_region_start += PAGES_IN_CHUNK;
            }
            debug_assert!(next_region_start < freelist::MAX_UNITS as usize);
            if pages_freed == next_region_start - region_start {
                let start = sync.start;
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
