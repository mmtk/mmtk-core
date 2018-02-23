use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::marker::PhantomData;

use ::util::address::Address;
use ::policy::space::Space;
use super::vmrequest::HEAP_LAYOUT_64BIT;
use super::layout::vm_layout_constants::BYTES_IN_CHUNK;

use ::util::heap::pageresource::CommonPageResource;
use ::util::alloc::embedded_meta_data::*;

use super::PageResource;

const SPACE_ALIGN: usize = 1 << 19;

#[derive(Debug)]
pub struct MonotonePageResource<S: Space<MonotonePageResource<S>>> where S: 'static {
    common: CommonPageResource<MonotonePageResource<S>, S>,

    /** Pointer to the next block to be allocated. */
    cursor: Address,
    /** The limit of the currently allocated address space. */
    sentinel: Address,
    /** Number of pages to reserve at the start of every allocation */
    meta_data_pages_per_region: usize,
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

impl<S: Space<MonotonePageResource<S>>> PageResource<S> for MonotonePageResource<S> {
    fn common(&self) -> &CommonPageResource<Self, S> {
        &self.common
    }

    fn common_mut(&mut self) -> &mut CommonPageResource<Self, S> {
        &mut self.common
    }

    fn alloc_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool) -> Address {
        unimplemented!()
        /*let mut new_chunk = false;
        let guard = self.common().lock.lock().unwrap();
        let mut rtn = self.cursor;

        if cfg!("debug") {
            /*
             * Cursor should always be zero, or somewhere in the current chunk.  If we have just
             * allocated exactly enough pages to exhaust the current chunk, then cursor can point
             * to the next chunk.
             */
            if self.current_chunk > self.cursor
                || (chunk_align!(self.cursor, true).as_usize() != self.current_chunk.as_usize()
                    && chunk_align!(self.cursor, true) != self.current_chunk + BYTES_IN_CHUNK) {
                log_chunk_fields("MonotonePageResource.alloc_pages:fail");
            }
            assert!(self.current_chunk <= self.cursor);
            assert!(self.cursor.is_zero() ||
                chunk_align!(self.cursor.as_usize(), true) == self.current_chunk.as_usize() ||
                chunk_align!(self.cursor, true).as_usize() == (self.current_chunk + BYTES_IN_CHUNK)
                    .as_usize());
        }

        if self.meta_data_pages_per_region != 0 {
            /* adjust allocation for metadata */
            let region_start = get_region_start(self.cursor + pages_to_bytes(required_pages));
            let region_delta = region_start - self.cursor;
            if (regionDelta.sGE(Offset.zero())) {
                /* start new region, so adjust pages and return address accordingly */
                requiredPages += Conversions.bytesToPages(regionDelta) + metaDataPagesPerRegion;
                rtn = regionStart.plus(Conversions.pagesToBytes(metaDataPagesPerRegion));
            }
        }
        Extent bytes = Conversions.pagesToBytes(requiredPages);
        Address tmp = cursor.plus(bytes);

        if (!contiguous && tmp.GT(sentinel)) {
            /* we're out of virtual memory within our discontiguous region, so ask for more */
            int requiredChunks = Space.requiredChunks(requiredPages);
            currentChunk = space.growDiscontiguousSpace(requiredChunks); // Returns zero on failure
            cursor = currentChunk;
            sentinel = cursor.plus(currentChunk.isZero() ? 0 : requiredChunks << VMLayoutConstants.LOG_BYTES_IN_CHUNK);
            rtn = cursor;
            tmp = cursor.plus(bytes);
            newChunk = true;
        }
        if (VM.VERIFY_ASSERTIONS)
        VM.assertions._assert(rtn.GE(cursor) && rtn.LT(cursor.plus(bytes)));
        if (tmp.GT(sentinel)) {
            unlock();
            return Address.zero();
        } else {
            Address old = cursor;
            cursor = tmp;

            /* In a contiguous space we can bump along into the next chunk, so preserve the currentChunk invariant */
            if (contiguous && Conversions.chunkAlign(cursor, true).NE(currentChunk)) {
                currentChunk = Conversions.chunkAlign(cursor, true);
            }
            commitPages(reservedPages, requiredPages);
            space.growSpace(old, bytes, newChunk);
            unlock();
            HeapLayout.mmapper.ensureMapped(old, requiredPages);
            if (zeroed) {
                if (!zeroConcurrent) {
                    VM.memory.zero(zeroNT, old, bytes);
                } else {
                    while (cursor.GT(zeroingCursor));
                }
            }
            VM.events.tracePageAcquired(space, rtn, requiredPages);
            return rtn;
        }*/
    }

    fn adjust_for_metadata(&self, pages: usize) -> usize {
        pages + ((pages + PAGES_IN_REGION - 1) >> LOG_PAGES_IN_REGION)
            * self.meta_data_pages_per_region
    }
}

impl<S: Space<MonotonePageResource<S>>> MonotonePageResource<S> {
    pub fn new_contiguous(start: Address, bytes: usize,
                          meta_data_pages_per_region: usize) -> Self {
        let sentinel = start + bytes;

        MonotonePageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                lock: Mutex::new(()),
                contiguous: false,
                growable: HEAP_LAYOUT_64BIT,
                space: None,
                _placeholder: PhantomData,
            },

            cursor: start,
            current_chunk: unsafe{Address::from_usize(chunk_align!(start.as_usize(), true))},
            sentinel,
            meta_data_pages_per_region,
            conditional: MonotonePageResourceConditional::Contiguous {
                start,
                zeroing_cursor: sentinel,
                zeroing_sentinel: start,
            },
        }
    }

    pub fn new_discontiguous(meta_data_pages_per_region: usize) -> Self {
        MonotonePageResource {
            common: CommonPageResource {
                reserved: AtomicUsize::new(0),
                committed: AtomicUsize::new(0),
                lock: Mutex::new(()),
                contiguous: false,
                growable: true,
                space: None,
                _placeholder: PhantomData,
            },

            cursor: unsafe { Address::zero() },
            current_chunk: unsafe { Address::zero() },
            sentinel: unsafe { Address::zero() },
            meta_data_pages_per_region,
            conditional: MonotonePageResourceConditional::Discontiguous,
        }
    }

    pub fn reset(&mut self) {
        unimplemented!()
    }
}