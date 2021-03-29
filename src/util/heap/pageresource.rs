use crate::util::address::Address;
use crate::util::conversions;
use crate::util::OpaquePointer;
use crate::vm::ActivePlan;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::layout::map::Map;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::space_descriptor::SpaceDescriptor;
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
        zeroed: bool,
        tls: OpaquePointer,
    ) -> Result<PRAllocResult, PRAllocFail> {
        self.alloc_pages(
            space_descriptor,
            reserved_pages,
            required_pages,
            zeroed,
            tls,
        )
    }

    // XXX: In the original code reserve_pages & clear_request explicitly
    //      acquired a lock.
    fn reserve_pages(&self, pages: usize) -> usize {
        let adj_pages = self.adjust_for_metadata(pages);
        self.common()
            .reserved
            .fetch_add(adj_pages, Ordering::Relaxed);
        adj_pages
    }

    fn clear_request(&self, reserved_pages: usize) {
        self.common()
            .reserved
            .fetch_sub(reserved_pages, Ordering::Relaxed);
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
        zeroed: bool,
        tls: OpaquePointer,
    ) -> Result<PRAllocResult, PRAllocFail>;

    fn adjust_for_metadata(&self, pages: usize) -> usize;

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
    fn commit_pages(&self, reserved_pages: usize, actual_pages: usize, tls: OpaquePointer) {
        let delta = actual_pages - reserved_pages;
        self.common().reserved.fetch_add(delta, Ordering::Relaxed);
        self.common()
            .committed
            .fetch_add(actual_pages, Ordering::Relaxed);
        if unsafe { VM::VMActivePlan::is_mutator(tls) } {
            self.vm_map()
                .add_to_cumulative_committed_pages(actual_pages);
        }
    }

    fn reserved_pages(&self) -> usize {
        self.common().reserved.load(Ordering::Relaxed)
    }

    fn committed_pages(&self) -> usize {
        self.common().committed.load(Ordering::Relaxed)
    }

    fn common(&self) -> &CommonPageResource;
    fn common_mut(&mut self) -> &mut CommonPageResource;
    fn vm_map(&self) -> &'static VMMap {
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
    reserved: AtomicUsize,
    committed: AtomicUsize,

    pub contiguous: bool,
    pub growable: bool,

    vm_map: &'static VMMap,
    head_discontiguous_region: Mutex<Address>,
}

impl CommonPageResource {
    pub fn new(contiguous: bool, growable: bool, vm_map: &'static VMMap) -> CommonPageResource {
        CommonPageResource {
            reserved: AtomicUsize::new(0),
            committed: AtomicUsize::new(0),

            contiguous,
            growable,
            vm_map,

            head_discontiguous_region: Mutex::new(Address::ZERO),
        }
    }

    pub fn reserve(&self, pages: usize) {
        self.reserved.fetch_add(pages, Ordering::Relaxed);
    }

    pub fn release_reserved(&self, pages: usize) {
        self.reserved.fetch_sub(pages, Ordering::Relaxed);
    }

    pub fn get_reserved(&self) -> usize {
        self.reserved.load(Ordering::Relaxed)
    }

    pub fn reset_reserved(&self) {
        self.reserved.store(0, Ordering::Relaxed);
    }

    pub fn commit(&self, pages: usize) {
        self.committed.fetch_add(pages, Ordering::Relaxed);
    }

    pub fn release_committed(&self, pages: usize) {
        self.committed.fetch_sub(pages, Ordering::Relaxed);
    }

    pub fn get_committed(&self) -> usize {
        self.committed.load(Ordering::Relaxed)
    }

    pub fn reset_committed(&self) {
        self.committed.store(0, Ordering::Relaxed);
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

        let new_head: Address = self.vm_map.allocate_contiguous_chunks(
            space_descriptor,
            chunks,
            *head_discontiguous_region,
        );
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
        self.vm_map.free_contiguous_chunks(chunk);
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
