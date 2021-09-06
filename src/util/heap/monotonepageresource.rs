use super::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::policy::space::required_chunks;
use crate::util::address::Address;
use crate::util::conversions::*;
use std::sync::{Mutex, MutexGuard};

use crate::util::alloc::embedded_meta_data::*;
use crate::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;
use crate::util::heap::pageresource::CommonPageResource;
use crate::util::opaque_pointer::*;

use super::layout::map::Map;
use super::pageresource::{PRAllocFail, PRAllocResult};
use super::PageResource;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::vm::VMBinding;
use std::marker::PhantomData;

pub struct MonotonePageResource<VM: VMBinding> {
    common: CommonPageResource,

    /** Number of pages to reserve at the start of every allocation */
    meta_data_pages_per_region: usize,
    sync: Mutex<MonotonePageResourceSync>,
    _p: PhantomData<VM>,
}

struct MonotonePageResourceSync {
    /** Pointer to the next block to be allocated. */
    cursor: Address,
    /** The limit of the currently allocated address space. */
    sentinel: Address,
    /** Base address of the current chunk of addresses */
    current_chunk: Address,
    conditional: MonotonePageResourceConditional,
}

pub enum MonotonePageResourceConditional {
    Contiguous {
        start: Address,
        /** Current frontier of zeroing, in a separate zeroing thread */
        zeroing_cursor: Address,
        /** Current limit of zeroing.  If zeroingCursor < zeroingSentinel, zeroing is still happening. */
        zeroing_sentinel: Address,
    },
    Discontiguous,
}
impl<VM: VMBinding> PageResource<VM> for MonotonePageResource<VM> {
    fn common(&self) -> &CommonPageResource {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource {
        &mut self.common
    }

    fn reserve_pages(&self, pages: usize) -> usize {
        self.common().accounting.reserve(pages);
        pages
    }

    fn alloc_pages(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        immut_required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        debug!(
            "In MonotonePageResource, reserved_pages = {}, required_pages = {}",
            reserved_pages, immut_required_pages
        );
        let mut required_pages = immut_required_pages;
        let mut new_chunk = false;
        let mut sync = self.sync.lock().unwrap();
        let mut rtn = sync.cursor;
        debug!(
            "cursor = {}, sentinel = {}, current_chunk = {}",
            sync.cursor, sync.sentinel, sync.current_chunk
        );

        if cfg!(debug = "true") {
            /*
             * Cursor should always be zero, or somewhere in the current chunk.  If we have just
             * allocated exactly enough pages to exhaust the current chunk, then cursor can point
             * to the next chunk.
             */
            if sync.current_chunk > sync.cursor
                || (chunk_align_down(sync.cursor) != sync.current_chunk
                    && chunk_align_down(sync.cursor) != sync.current_chunk + BYTES_IN_CHUNK)
            {
                self.log_chunk_fields(space_descriptor, "MonotonePageResource.alloc_pages:fail");
            }
            assert!(sync.current_chunk <= sync.cursor);
            assert!(
                sync.cursor.is_zero()
                    || chunk_align_down(sync.cursor) == sync.current_chunk
                    || chunk_align_down(sync.cursor) == (sync.current_chunk + BYTES_IN_CHUNK)
            );
        }

        if self.meta_data_pages_per_region != 0 {
            /* adjust allocation for metadata */
            let region_start = Self::get_region_start(sync.cursor + pages_to_bytes(required_pages));
            let region_delta = region_start.get_offset(sync.cursor);
            if region_delta >= 0 {
                /* start new region, so adjust pages and return address accordingly */
                required_pages +=
                    bytes_to_pages(region_delta as usize) + self.meta_data_pages_per_region;
                rtn = region_start + pages_to_bytes(self.meta_data_pages_per_region);
            }
        }
        let bytes = pages_to_bytes(required_pages);
        debug!("bytes={}", bytes);
        let mut tmp = sync.cursor + bytes;
        debug!("tmp={:?}", tmp);

        if !self.common().contiguous && tmp > sync.sentinel {
            /* we're out of virtual memory within our discontiguous region, so ask for more */
            let required_chunks = required_chunks(required_pages);
            sync.current_chunk = self
                .common
                .grow_discontiguous_space(space_descriptor, required_chunks); // Returns zero on failure
            sync.cursor = sync.current_chunk;
            sync.sentinel = sync.cursor
                + if sync.current_chunk.is_zero() {
                    0
                } else {
                    required_chunks << LOG_BYTES_IN_CHUNK
                };
            //println!("{} {}->{}", self.common.space.unwrap().get_name(), sync.cursor, sync.sentinel);
            rtn = sync.cursor;
            tmp = sync.cursor + bytes;
            new_chunk = true;
        }

        debug_assert!(rtn >= sync.cursor && rtn < sync.cursor + bytes);
        if tmp > sync.sentinel {
            //debug!("tmp={:?} > sync.sentinel={:?}", tmp, sync.sentinel);
            Result::Err(PRAllocFail)
        } else {
            //debug!("tmp={:?} <= sync.sentinel={:?}", tmp, sync.sentinel);
            sync.cursor = tmp;
            debug!("update cursor = {}", tmp);

            /* In a contiguous space we can bump along into the next chunk, so preserve the currentChunk invariant */
            if self.common().contiguous && chunk_align_down(sync.cursor) != sync.current_chunk {
                sync.current_chunk = chunk_align_down(sync.cursor);
            }
            self.commit_pages(reserved_pages, required_pages, tls);

            Result::Ok(PRAllocResult {
                start: rtn,
                pages: required_pages,
                new_chunk,
            })
        }
    }

    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages
            + ((pages + PAGES_IN_REGION - 1) >> LOG_PAGES_IN_REGION)
                * self.meta_data_pages_per_region
    }
}

impl<VM: VMBinding> MonotonePageResource<VM> {
    pub fn new_contiguous(
        start: Address,
        bytes: usize,
        meta_data_pages_per_region: usize,
        vm_map: &'static VMMap,
    ) -> Self {
        let sentinel = start + bytes;

        MonotonePageResource {
            common: CommonPageResource::new(true, cfg!(target_pointer_width = "64"), vm_map),

            meta_data_pages_per_region,
            sync: Mutex::new(MonotonePageResourceSync {
                cursor: start,
                current_chunk: chunk_align_down(start),
                sentinel,
                conditional: MonotonePageResourceConditional::Contiguous {
                    start,
                    zeroing_cursor: sentinel,
                    zeroing_sentinel: start,
                },
            }),
            _p: PhantomData,
        }
    }

    pub fn new_discontiguous(meta_data_pages_per_region: usize, vm_map: &'static VMMap) -> Self {
        MonotonePageResource {
            common: CommonPageResource::new(false, true, vm_map),

            meta_data_pages_per_region,
            sync: Mutex::new(MonotonePageResourceSync {
                cursor: unsafe { Address::zero() },
                current_chunk: unsafe { Address::zero() },
                sentinel: unsafe { Address::zero() },
                conditional: MonotonePageResourceConditional::Discontiguous,
            }),
            _p: PhantomData,
        }
    }

    /// Get highwater mark of current monotone space.
    pub fn cursor(&self) -> Address {
        self.sync.lock().unwrap().cursor
    }

    fn log_chunk_fields(&self, space_descriptor: SpaceDescriptor, site: &str) {
        let sync = self.sync.lock().unwrap();
        debug!(
            "[{:?}]{}: cursor={}, current_chunk={}, delta={}",
            space_descriptor,
            site,
            sync.cursor,
            sync.current_chunk,
            sync.cursor - sync.current_chunk
        );
    }

    fn get_region_start(addr: Address) -> Address {
        addr.align_down(BYTES_IN_REGION)
    }

    /// # Safety
    /// TODO: I am not sure why this is unsafe.
    pub unsafe fn reset(&self) {
        let mut guard = self.sync.lock().unwrap();
        self.common().accounting.reset();
        self.release_pages(&mut guard);
        drop(guard);
    }

    pub unsafe fn get_current_chunk(&self) -> Address {
        let guard = self.sync.lock().unwrap();
        guard.current_chunk
    }

    /*/**
    * Release all pages associated with this page resource, optionally
    * zeroing on release and optionally memory protecting on release.
    */
     @Inline
     private void releasePages() {
     if (contiguous) {
     // TODO: We will perform unnecessary zeroing if the nursery size has decreased.
     if (zeroConcurrent) {
     // Wait for current zeroing to finish.
     while (zeroingCursor.LT(zeroingSentinel)) { }
     }
     // Reset zeroing region.
     if (cursor.GT(zeroingSentinel)) {
     zeroingSentinel = cursor;
     }
     zeroingCursor = start;
     cursor = start;
     currentChunk = Conversions.chunkAlign(start, true);
     } else { /* Not contiguous */
     if (!cursor.isZero()) {
     do {
     Extent bytes = cursor.diff(currentChunk).toWord().toExtent();
     releasePages(currentChunk, bytes);
     } while (moveToNextChunk());

     currentChunk = Address.zero();
     sentinel = Address.zero();
     cursor = Address.zero();
     space.releaseAllChunks();
     }
     }
     }*/

    #[inline]
    unsafe fn release_pages(&self, guard: &mut MutexGuard<MonotonePageResourceSync>) {
        // TODO: concurrent zeroing
        if self.common().contiguous {
            guard.cursor = match guard.conditional {
                MonotonePageResourceConditional::Contiguous { start: _start, .. } => _start,
                _ => unreachable!(),
            };
        } else if !guard.cursor.is_zero() {
            let bytes = guard.cursor - guard.current_chunk;
            self.release_pages_extent(guard.current_chunk, bytes);
            while self.move_to_next_chunk(guard) {
                let bytes = guard.cursor - guard.current_chunk;
                self.release_pages_extent(guard.current_chunk, bytes);
            }

            guard.current_chunk = Address::zero();
            guard.sentinel = Address::zero();
            guard.cursor = Address::zero();
            self.common.release_all_chunks();
        }
    }

    fn release_pages_extent(&self, _first: Address, bytes: usize) {
        let pages = crate::util::conversions::bytes_to_pages(bytes);
        debug_assert!(bytes == crate::util::conversions::pages_to_bytes(pages));
        // FIXME ZERO_PAGES_ON_RELEASE
        // FIXME Options.protectOnRelease
        // FIXME VM.events.tracePageReleased
    }

    fn move_to_next_chunk(&self, guard: &mut MutexGuard<MonotonePageResourceSync>) -> bool {
        guard.current_chunk = self
            .vm_map()
            .get_next_contiguous_region(guard.current_chunk);
        if guard.current_chunk.is_zero() {
            false
        } else {
            guard.cursor = guard.current_chunk
                + self
                    .vm_map()
                    .get_contiguous_region_size(guard.current_chunk);
            true
        }
    }
}
