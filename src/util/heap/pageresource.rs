use crate::util::address::Address;
use crate::policy::space::Space;
use crate::vm::ActivePlan;
use crate::util::OpaquePointer;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::util::heap::layout::heap_layout::VMMap;
use crate::vm::VMBinding;

pub trait PageResource<VM: VMBinding>: Sized + 'static {
    type Space: Space<VM, PR = Self>;

    /// Allocate pages from this resource.
    /// Simply bump the cursor, and fail if we hit the sentinel.
    /// Return The start of the first page if successful, zero on failure.
    fn get_new_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool, tls: OpaquePointer) -> Address {
        self.alloc_pages(reserved_pages, required_pages, zeroed, tls)
    }

    // XXX: In the original code reserve_pages & clear_request explicitly
    //      acquired a lock.
    fn reserve_pages(&self, pages: usize) -> usize {
        let adj_pages = self.adjust_for_metadata(pages);
        self.common().reserved.fetch_add(adj_pages, Ordering::Relaxed);
        adj_pages
    }

    fn clear_request(&self, reserved_pages: usize) {
        self.common().reserved.fetch_sub(reserved_pages, Ordering::Relaxed);
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

    fn alloc_pages(&self, reserved_pages: usize, required_pages: usize, zeroed: bool, tls: OpaquePointer) -> Address;

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
        self.common().committed.fetch_add(actual_pages, Ordering::Relaxed);
        if unsafe{VM::VMActivePlan::is_mutator(tls)} {
            self.vm_map().add_to_cumulative_committed_pages(actual_pages);
        }
    }

    fn reserved_pages(&self) -> usize {
        self.common().reserved.load(Ordering::Relaxed)
    }

    fn committed_pages(&self) -> usize {
        self.common().committed.load(Ordering::Relaxed)
    }

    fn bind_space(&mut self, space: &'static Self::Space) {
        self.common_mut().space = Some(space);
    }

    fn common(&self) -> &CommonPageResource<VM, Self>;
    fn common_mut(&mut self) -> &mut CommonPageResource<VM, Self>;
    fn vm_map(&self) -> &'static VMMap {
        self.common().space.unwrap().common().vm_map()
    }
}

pub struct CommonPageResource<VM: VMBinding, PR: PageResource<VM>> {
    pub reserved: AtomicUsize,
    pub committed: AtomicUsize,

    pub contiguous: bool,
    pub growable: bool,
    pub space: Option<&'static PR::Space>,
}