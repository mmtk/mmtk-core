use super::gc_works::MSProcessEdges;
use crate::mmtk::MMTK;
use crate::policy::malloc::*;
use crate::plan::global::NoCopy;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::marksweep::mutator::create_ms_mutator;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::{Address, ObjectReference, OpaquePointer};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::vm::VMBinding;
use std::{ops::Sub, sync::Arc};

use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

pub struct MarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for MarkSweep<VM> {}

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = NoCopy<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<Self::VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        MarkSweep {
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }



    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool
    where
            Self: Sized, {
        unsafe { malloc_memory_full() }
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop and scan mutators
        scheduler
            .unconstrained_works
            .add(StopMutators::<MSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.final_stage.add(ScheduleSanityGC);
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        _mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_ms_mutator(tls, self))
    }
    
    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer) {
        // Do nothing  
    }

    fn release(&self, tls: OpaquePointer) {
        unsafe {

            //using hashset
            let mut NODES_mut = &mut *NODES.lock().unwrap();
            NODES_mut.retain(|&o| MarkSweep::<VM>::marked(&o));
            for object in NODES_mut.iter() {
                let a: Address = object.to_address().sub(8);
                let marking_word: usize = a.load();
                debug_assert!(marking_word != 0usize, "Marking word is 0, should have been removed from NODES");
                debug_assert!(marking_word == 1usize, "Marking word must be 1 or 0, found {}", marking_word);
                a.store(0);
            }

            //using bitmaps 
            // let mut MALLOCED_mut = &mut *MALLOCED.lock().unwrap();
            // let mut MARKED_mut = &mut *MARKED.lock().unwrap();
            // let mut to_free = MALLOCED_mut.clone();
            // to_free.xor(MARKED_mut);
            // let count: usize = 0;
            // let max = to_free.len();

            // while count < max {
            //     if to_free.get(count).unwrap() {
            //         MALLOCED_mut.set(count, false);
            //         MARKED_mut.set(count, false);
            //         let object = index_to_object_reference(count);
            //         libc::free(object.to_address().to_mut_ptr());
            //     }
            // }
        }
    }

    fn get_collection_reserve(&self) -> usize {
        unimplemented!();
    }

    fn get_pages_used(&self) -> usize {
        let mem = MEMORY_ALLOCATED.lock().unwrap();
        MALLOC_MEMORY - *mem
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    fn marked(&object: &ObjectReference) -> bool {
        unsafe {
            let address: Address = object.to_address().sub(8);
            let marking_word: usize = address.load();
            let mut mem = MEMORY_ALLOCATED.lock().unwrap();
            if marking_word == 0 {
                let obj_size = libc::malloc_usable_size(address.to_mut_ptr());
                debug_assert!(*mem >= obj_size, "Attempting to free an object sized {} when total memory allocated is {}", obj_size, mem);
                *mem -= obj_size;
                debug_assert!(*mem >= 0, "amount of memory allocated cannot be negative!");
                libc::free(address.to_mut_ptr()); 
                return false
            }
            true
        }
    }
}
