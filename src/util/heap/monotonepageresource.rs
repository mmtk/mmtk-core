use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::marker::PhantomData;

use ::util::address::Address;
use ::util::conversions::*;
use ::policy::space::Space;
use ::policy::space::required_chunks;
use super::vmrequest::HEAP_LAYOUT_64BIT;
use super::layout::vm_layout_constants::BYTES_IN_CHUNK;

use ::util::heap::pageresource::CommonPageResource;
use ::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;
use ::util::alloc::embedded_meta_data::*;

use super::layout::Mmapper;
use super::layout::heap_layout::MMAPPER;

use super::PageResource;
use std::sync::atomic::Ordering;

const SPACE_ALIGN: usize = 1 << 19;

#[derive(Debug)]
pub struct MonotonePageResource<S: Space<PR = MonotonePageResource<S>>> {
    common: CommonPageResource<MonotonePageResource<S>>,

    /** Number of pages to reserve at the start of every allocation */
    meta_data_pages_per_region: usize,
    sync: Mutex<MonotonePageResourceSync>,
}

#[derive(Debug)]
struct MonotonePageResourceSync {
    /** Pointer to the next block to be allocated. */
    cursor: Address,
    /** The limit of the currently allocated address space. */
    sentinel: Address,
    /** Base address of the current chunk of addresses */
    current_chunk: Address,
    conditional: MonotonePageResourceConditional,
}

#[derive(Debug)]
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
impl<S: Space<PR = MonotonePageResource<S>>> PageResource for MonotonePageResource<S> {
    type Space = S;

    fn common(&self) -> &CommonPageResource<Self> {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonPageResource<Self> {
        &mut self.common
    }

    fn alloc_pages(&self, reserved_pages: usize, immut_required_pages: usize, zeroed: bool,
                   thread_id: usize) -> Address {
        let mut required_pages = immut_required_pages;
        let mut new_chunk = false;
        let mut sync = self.sync.lock().unwrap();
        let mut rtn = sync.cursor;

        if cfg!(debug = "true") {
            /*
             * Cursor should always be zero, or somewhere in the current chunk.  If we have just
             * allocated exactly enough pages to exhaust the current chunk, then cursor can point
             * to the next chunk.
             */
            if sync.current_chunk > sync.cursor
                || (chunk_align(sync.cursor, true).as_usize() != sync.current_chunk.as_usize()
                    && chunk_align(sync.cursor, true).as_usize() != sync.current_chunk.as_usize()
                        + BYTES_IN_CHUNK) {
                self.log_chunk_fields("MonotonePageResource.alloc_pages:fail");
            }
            assert!(sync.current_chunk <= sync.cursor);
            assert!(sync.cursor.is_zero() ||
                chunk_align(sync.cursor, true).as_usize() == sync.current_chunk.as_usize() ||
                chunk_align(sync.cursor, true).as_usize() == (sync.current_chunk + BYTES_IN_CHUNK)
                    .as_usize());
        }

        if self.meta_data_pages_per_region != 0 {
            /* adjust allocation for metadata */
            let region_start = Self::get_region_start(sync.cursor + pages_to_bytes(required_pages));
            let region_delta = region_start.as_usize() as isize - sync.cursor.as_usize() as isize;
            if region_delta >= 0 {
                /* start new region, so adjust pages and return address accordingly */
                required_pages += bytes_to_pages(region_delta as usize) + self.meta_data_pages_per_region;
                rtn = region_start + pages_to_bytes(self.meta_data_pages_per_region);
            }
        }
        let bytes = pages_to_bytes(required_pages);
        trace!("bytes={}", bytes);
        let mut tmp = sync.cursor + bytes;
        trace!("tmp={:?}", tmp);

        if !self.common().contiguous && tmp > sync.sentinel {
            /* we're out of virtual memory within our discontiguous region, so ask for more */
            let required_chunks = required_chunks(required_pages);
            sync.current_chunk = unsafe {
                self.common().space.unwrap().grow_discontiguous_space(required_chunks)
            }; // Returns zero on failure
            sync.cursor = sync.current_chunk;
            sync.sentinel = sync.cursor + if sync.current_chunk.is_zero() { 0 } else {
                required_chunks << LOG_BYTES_IN_CHUNK };
            rtn = sync.cursor;
            tmp = sync.cursor + bytes;
            new_chunk = true;
        }

        debug_assert!(rtn >= sync.cursor && rtn < sync.cursor + bytes);
        if tmp > sync.sentinel {
            trace!("tmp={:?} > sync.sentinel={:?}", tmp, sync.sentinel);
            return unsafe{Address::zero()};
        } else {
            let old = sync.cursor;
            sync.cursor = tmp;

            /* In a contiguous space we can bump along into the next chunk, so preserve the currentChunk invariant */
            if self.common().contiguous && chunk_align(sync.cursor, true).as_usize() != sync.current_chunk.as_usize() {
                sync.current_chunk = chunk_align(sync.cursor, true);
            }
            self.commit_pages(reserved_pages, required_pages, thread_id);
            self.common().space.unwrap().grow_space(old, bytes, new_chunk);

            MMAPPER.ensure_mapped(old, required_pages);

            // FIXME: Zeroing
            /*
            if zeroed {
                if !self.zero_concurrent {
                    VM.memory.zero(zeroNT, old, bytes);
                } else {
                    while (cursor.GT(zeroingCursor));
                }
            }
            VM.events.tracePageAcquired(space, rtn, requiredPages);
            */
            rtn
        }
    }

    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages + ((pages + PAGES_IN_REGION - 1) >> LOG_PAGES_IN_REGION)
            * self.meta_data_pages_per_region
    }
}

impl<S: Space<PR = MonotonePageResource<S>>> MonotonePageResource<S> {
    pub fn new_contiguous(start: Address, bytes: usize,
                          meta_data_pages_per_region: usize) -> Self {
        let sentinel = start + bytes;

        MonotonePageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                contiguous: true,
                growable: HEAP_LAYOUT_64BIT,
                space: None,
            },

            meta_data_pages_per_region,
            sync: Mutex::new(MonotonePageResourceSync {
                cursor: start,
                current_chunk: unsafe{Address::from_usize(chunk_align!(start.as_usize(), true))},
                sentinel,
                conditional: MonotonePageResourceConditional::Contiguous {
                    start,
                    zeroing_cursor: sentinel,
                    zeroing_sentinel: start,
                }
            }),
        }
    }

    pub fn new_discontiguous(meta_data_pages_per_region: usize) -> Self {
        MonotonePageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                contiguous: false,
                growable: true,
                space: None,
            },

            meta_data_pages_per_region,
            sync: Mutex::new(MonotonePageResourceSync {
                cursor: unsafe { Address::zero() },
                current_chunk: unsafe { Address::zero() },
                sentinel: unsafe { Address::zero() },
                conditional: MonotonePageResourceConditional::Discontiguous,
            }),
        }
    }

    fn log_chunk_fields(&self, site: &str) {
        let sync = self.sync.lock().unwrap();
        debug!("[{}]{}: cursor={}, current_chunk={}, delta={}",
               self.common().space.unwrap().common().name,
               site, sync.cursor.as_usize(), sync.current_chunk, sync.cursor - sync.current_chunk);
    }

    fn get_region_start(addr: Address) -> Address {
        unsafe{
            Address::from_usize(addr.as_usize() & !(BYTES_IN_REGION - 1))
        }
    }

    pub fn reset(&self) {
        let guard = self.sync.lock().unwrap();
        self.common().reserved.store(0, Ordering::Relaxed);
        self.common().committed.store(0, Ordering::Relaxed);
        drop(guard);
    }
}