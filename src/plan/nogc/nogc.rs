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
use plan::plan::create_vm_space;
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub control_collector_context: ControllerCollectorContext,
    pub unsync: UnsafeCell<NoGCUnsync>,
}

unsafe impl Sync for NoGC {}

pub struct NoGCUnsync {
    vm_space: ImmortalSpace,
    pub space: ImmortalSpace,
    pub los: LargeObjectSpace,
    pub mmapper: &'static Mmapper,
    pub total_pages: usize,
}

impl Plan for NoGC {
    type MutatorT = NoGCMutator;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> Self {
        NoGC {
            control_collector_context: ControllerCollectorContext::new(),
            unsync: UnsafeCell::new(NoGCUnsync {
                vm_space: create_vm_space(vm_map, mmapper),
                space: ImmortalSpace::new("nogc_space", true,
                                          VMRequest::discontiguous(), vm_map, mmapper),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous(), vm_map, mmapper),
                mmapper,
                total_pages: 0,
            }
            ),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        // FIXME correctly initialize spaces based on options
        unsync.vm_space.init(vm_map);
        unsync.space.init(vm_map);
        unsync.los.init(vm_map);

        // These VMs require that the controller thread is started by the VM itself.
        // (Usually because it calls into VM code that accesses the TLS.)
        if !(cfg!(feature = "jikesrvm") || cfg!(feature = "openjdk")) {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(OpaquePointer::UNINITIALIZED )
            });
        }
    }

    fn mmapper(&self) -> &'static Mmapper {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.mmapper
    }

    fn bind_mutator(&self, tls: OpaquePointer) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(NoGCMutator::new(
            tls, &unsync.space, &unsync.los))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase) {}

    fn get_total_pages(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.total_pages
    }

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
            return unsync.mmapper.address_is_mapped(address);
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