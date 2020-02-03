use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;
use ::policy::largeobjectspace::LargeObjectSpace;
use ::plan::controller_collector_context::ControllerCollectorContext;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;
use ::util::heap::VMRequest;
use ::util::heap::layout::Mmapper as IMmapper;
use ::util::Address;
use ::util::OpaquePointer;

use std::cell::UnsafeCell;
use std::thread;
use libc::c_void;

use std::mem::uninitialized;

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;
use util::conversions::bytes_to_pages;
use plan::plan::{create_vm_space, CommonPlan};
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;
use util::options::{Options, UnsafeOptionsWrapper};
use std::sync::Arc;
use util::heap::HeapMeta;
use util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END};
use std::sync::atomic::Ordering;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub unsync: UnsafeCell<NoGCUnsync>,
    pub common: CommonPlan,
}

unsafe impl Sync for NoGC {}

pub struct NoGCUnsync {
    vm_space: ImmortalSpace,
    pub space: ImmortalSpace,
    pub los: LargeObjectSpace,
}

impl Plan for NoGC {
    type MutatorT = NoGCMutator;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        NoGC {
            unsync: UnsafeCell::new(NoGCUnsync {
                vm_space: create_vm_space(vm_map, mmapper, &mut heap),
                space: ImmortalSpace::new("nogc_space", true,
                                          VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
            }),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map(self.common.heap.get_discontig_start(), self.common.heap.get_discontig_end());

        let unsync = &mut *self.unsync.get();
        self.common.heap.total_pages.store(bytes_to_pages(heap_size), Ordering::Relaxed);
        // FIXME correctly initialize spaces based on options
        unsync.vm_space.init(vm_map);
        unsync.space.init(vm_map);
        unsync.los.init(vm_map);
    }

    fn common(&self) -> &CommonPlan {
        &self.common
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> *mut c_void {
        Box::into_raw(Box::new(NoGCMutator::new(tls, self))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase) {}

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.space.reserved_pages() + unsync.los.reserved_pages()
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            return true;
        }
        if unsync.vm_space.in_space(object) {
            return true;
        }
        if unsync.los.in_space(object) {
            return true;
        }
        return false;
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        false
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe {
            unsync.space.in_space(address.to_object_reference()) ||
            unsync.vm_space.in_space(address.to_object_reference()) ||
            unsync.los.in_space(address.to_object_reference())
        } {
            return self.common.mmapper.address_is_mapped(address);
        } else {
            return false;
        }
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            return unsync.space.is_movable();
        }
        if unsync.vm_space.in_space(object) {
            return unsync.vm_space.is_movable();
        }
        if unsync.los.in_space(object) {
            return unsync.los.is_movable();
        }
        return true;
    }
}

impl NoGC {
    pub fn get_immortal_space(&self) -> &'static ImmortalSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.space
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.los
    }
}