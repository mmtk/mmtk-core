use crate::plan::{Phase, Plan};
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::space::Space;
use crate::util::heap::layout::Mmapper as IMmapper;
use crate::util::heap::VMRequest;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;

use std::cell::UnsafeCell;

use super::NoGCCollector;
use super::NoGCMutator;
use super::NoGCTraceLocal;
use crate::plan::plan::CommonPlan;
use crate::util::conversions::bytes_to_pages;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub type SelectedPlan<VM> = NoGC<VM>;

pub struct NoGC<VM: VMBinding> {
    pub unsync: UnsafeCell<NoGCUnsync<VM>>,
    pub common: CommonPlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for NoGC<VM> {}

pub struct NoGCUnsync<VM: VMBinding> {
    pub space: ImmortalSpace<VM>,
}

impl<VM: VMBinding> Plan<VM> for NoGC<VM> {
    type MutatorT = NoGCMutator<VM>;
    type TraceLocalT = NoGCTraceLocal<VM>;
    type CollectorT = NoGCCollector<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        NoGC {
            unsync: UnsafeCell::new(NoGCUnsync {
                space: ImmortalSpace::new(
                    "nogc_space",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                ),
            }),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map(
            self.common.heap.get_discontig_start(),
            self.common.heap.get_discontig_end(),
        );

        self.common
            .heap
            .total_pages
            .store(bytes_to_pages(heap_size), Ordering::Relaxed);
        // FIXME correctly initialize spaces based on options
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.space.init(vm_map);
        self.common.gc_init(heap_size, vm_map)
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<NoGCMutator<VM>> {
        Box::new(NoGCMutator::new(tls, self))
    }

    fn will_never_move(&self, _object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, _tls: OpaquePointer, _phase: &Phase) {
        unreachable!()
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.space.reserved_pages()
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            true
        } else {
            self.common.is_valid_ref(object)
        }
    }

    fn is_bad_ref(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe { unsync.space.in_space(address.to_object_reference()) } {
            self.common.mmapper.address_is_mapped(address)
        } else {
            self.common.is_mapped_address(address)
        }
    }

    fn is_movable(&self, _object: ObjectReference) -> bool {
        false // By definition no objects are movable in NoGC
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn get_immortal_space(&self) -> &'static ImmortalSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.space
    }
}
