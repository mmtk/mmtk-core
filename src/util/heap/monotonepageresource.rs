use ::util::address::Address;
use std::ptr::null_mut;
use ::policy::space::Space;
use super::vmrequest::HEAP_LAYOUT_64BIT;
use super::layout::vm_layout_constants::BYTES_IN_CHUNK;

use super::PageResource;

const SPACE_ALIGN: usize = 1 << 19;

#[derive(Debug)]
pub struct MonotonePageResource<'a, S: Space<'a, MonotonePageResource<'a, S>>> {
    reserved: usize,
    committed: usize,
    growable: bool,
    space: Option<&'a S>,

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

impl<'a, S: Space<'a, MonotonePageResource<'a, S>>> PageResource<'a, Space<'a, MonotonePageResource<'a, S>>> for MonotonePageResource<'a, S> {
    fn reserve_pages(&self, pages: usize) -> usize {
        unimplemented!()
    }

    fn clear_request(&self, reserved_pages: usize) {
        unimplemented!()
    }

    fn update_zeroing_approach(&self, nontemporal: bool, concurrent: bool) {
        unimplemented!()
    }

    fn skip_concurrent_zeroing(&self) {
        unimplemented!()
    }

    fn trigger_concurrent_zeroing(&self) {
        unimplemented!()
    }

    fn concurrent_zeroing(&self) {
        unimplemented!()
    }

    fn alloc_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool) -> Address {
        unimplemented!()
    }

    fn adjust_for_metadata(&self, pages: usize) {
        unimplemented!()
    }

    fn commit_pages(&self, reserved_pages: usize, actual_pages: usize) {
        unimplemented!()
    }

    fn reserved_pages(&self) -> usize {
        unimplemented!()
    }

    fn committed_pages(&self) -> usize {
        unimplemented!()
    }

    fn cumulative_committed_pages() -> usize {
        unimplemented!()
    }


    fn bind_space(&mut self, space: &'static S) {
        self.space = Some(space);
    }
}

impl<'a, S: Space<'a, MonotonePageResource<'a, S>>> Drop for MonotonePageResource<'a, S> {
    fn drop(&mut self) {
        unimplemented!()
        /*let unmap_result = unsafe { munmap(self.mmap_start as *mut c_void, self.mmap_len) };
        if unmap_result != 0 {
            panic!("Failed to unmap {:?}", self);
        }*/
    }
}

impl<'a, S: Space<'a, MonotonePageResource<'a, S>>> MonotonePageResource<'a, S> {
    pub fn new_contiguous(start: Address, bytes: usize,
                          meta_data_pages_per_region: usize) -> Self {
        let sentinel = start + bytes;

        MonotonePageResource {
            reserved: 0,
            committed: 0,
            growable: HEAP_LAYOUT_64BIT,
            space: None,

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
            reserved: 0,
            committed: 0,
            growable: true,
            space: None,

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