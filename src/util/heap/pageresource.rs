use crate::util::address::Address;
use crate::util::conversions;
use crate::util::opaque_pointer::*;
use crate::vm::ActivePlan;
use std::sync::Mutex;

use super::layout::VMMap;
use crate::util::heap::space_descriptor::SpaceDescriptor;
use crate::util::heap::PageAccounting;
use crate::vm::VMBinding;

pub trait PageResource<VM: VMBinding>: 'static {
    /// Allocate pages from this resource.
    /// Simply bump the cursor, and fail if we hit the sentinel.
    /// Return The start of the first page if successful, zero on failure.
    fn get_new_pages(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail> {
        self.alloc_pages(space_descriptor, reserved_pages, required_pages, tls)
    }

    // XXX: In the original code reserve_pages & clear_request explicitly
    //      acquired a lock.
    fn reserve_pages(&self, pages: usize) -> usize {
        self.common().accounting.reserve(pages);
        pages
    }

    fn clear_request(&self, reserved_pages: usize) {
        self.common().accounting.clear_reserved(reserved_pages);
    }

    fn update_zeroing_approach(&self, _nontemporal: bool, concurrent: bool) {
        debug_assert!(!concurrent || self.common().contiguous);
        unimplemented!()
    }

    fn skip_concurrent_zeroing(&self) {
        unimplemented!()
    }

    fn trigger_concurrent_zeroing(&self) {
        unimplemented!()
    }

    fn concurrent_zeroing(&self) {
        panic!("This PageResource does not implement concurrent zeroing")
    }

    fn alloc_pages(
        &self,
        space_descriptor: SpaceDescriptor,
        reserved_pages: usize,
        required_pages: usize,
        tls: VMThread,
    ) -> Result<PRAllocResult, PRAllocFail>;

    /**
     * Commit pages to the page budget.  This is called after
     * successfully determining that the request can be satisfied by
     * both the page budget and virtual memory.  This simply accounts
     * for the discrepancy between <code>committed</code> and
     * <code>reserved</code> while the request was pending.
     *
     * This *MUST* be called by each PageResource during the
     * allocPages, and the caller must hold the lock.
     */
    fn commit_pages(&self, reserved_pages: usize, actual_pages: usize, tls: VMThread) {
        let delta = actual_pages - reserved_pages;
        self.common().accounting.reserve(delta);
        self.common().accounting.commit(actual_pages);
        if VM::VMActivePlan::is_mutator(tls) {
            self.vm_map()
                .add_to_cumulative_committed_pages(actual_pages);
        }
    }

    fn reserved_pages(&self) -> usize {
        self.common().accounting.get_reserved_pages()
    }

    fn committed_pages(&self) -> usize {
        self.common().accounting.get_committed_pages()
    }

    /// Return the number of available physical pages by this resource. This includes all pages
    /// currently unused by this resource. If the resource is using a discontiguous space, it also
    /// includes the currently unassigned discontiguous space.
    ///
    /// Note: This just considers physical pages (i.e. virtual memory pages allocated for use by
    /// this resource). This calculation is orthogonal to and does not consider any restrictions on
    /// the number of pages this resource may actually use at any time (i.e. the number of
    /// committed and reserved pages).
    ///
    /// Note: The calculation is made on the assumption that all space that could be assigned to
    /// this resource would be assigned to this resource (i.e. the unused discontiguous space could
    /// just as likely be assigned to another competing resource).
    fn get_available_physical_pages(&self) -> usize;

    fn common(&self) -> &CommonPageResource;
    fn common_mut(&mut self) -> &mut CommonPageResource;
    fn vm_map(&self) -> &'static dyn VMMap {
        self.common().vm_map
    }
}

pub struct PRAllocResult {
    pub start: Address,
    pub pages: usize,
    pub new_chunk: bool,
}

pub struct PRAllocFail;

pub struct CommonPageResource {
    pub accounting: PageAccounting,
    pub contiguous: bool,
    pub growable: bool,

    pub vm_map: &'static dyn VMMap,
    head_discontiguous_region: Mutex<Address>,
}

impl CommonPageResource {
    pub fn new(contiguous: bool, growable: bool, vm_map: &'static dyn VMMap) -> CommonPageResource {
        CommonPageResource {
            accounting: PageAccounting::new(),

            contiguous,
            growable,
            vm_map,

            head_discontiguous_region: Mutex::new(Address::ZERO),
        }
    }

    /// Extend the virtual memory associated with a particular discontiguous
    /// space.  This simply involves requesting a suitable number of chunks
    /// from the pool of chunks available to discontiguous spaces.
    pub fn grow_discontiguous_space(
        &self,
        space_descriptor: SpaceDescriptor,
        chunks: usize,
    ) -> Address {
        let mut head_discontiguous_region = self.head_discontiguous_region.lock().unwrap();

        let new_head: Address = unsafe {
            self.vm_map.allocate_contiguous_chunks(
                space_descriptor,
                chunks,
                *head_discontiguous_region,
            )
        };
        if new_head.is_zero() {
            return Address::ZERO;
        }

        *head_discontiguous_region = new_head;
        new_head
    }

    /// Release one or more contiguous chunks associated with a discontiguous
    /// space.
    pub fn release_discontiguous_chunks(&self, chunk: Address) {
        let mut head_discontiguous_region = self.head_discontiguous_region.lock().unwrap();
        debug_assert!(chunk == conversions::chunk_align_down(chunk));
        if chunk == *head_discontiguous_region {
            *head_discontiguous_region = self.vm_map.get_next_contiguous_region(chunk);
        }
        unsafe {
            self.vm_map.free_contiguous_chunks(chunk);
        }
    }

    pub fn release_all_chunks(&self) {
        let mut head_discontiguous_region = self.head_discontiguous_region.lock().unwrap();
        self.vm_map.free_all_chunks(*head_discontiguous_region);
        *head_discontiguous_region = Address::ZERO;
    }

    pub fn get_head_discontiguous_region(&self) -> Address {
        *self.head_discontiguous_region.lock().unwrap()
    }
}
