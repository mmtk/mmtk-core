use super::gc_works::{MSProcessEdges};
use crate::{mmtk::MMTK, plan::global::NoCopy, util::ObjectReference};
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
use crate::util::heap::VMRequest;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::{sync::atomic::{AtomicBool, Ordering}};
use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashSet;
use crate::policy::space::NODES;

use enum_map::EnumMap;

pub type SelectedPlan<VM> = MarkSweep<VM>;

//pub const ALLOC_MS: AllocationSemantics = AllocationSemantics::Default;

pub struct MarkSweep<VM: VMBinding> {
    pub common: CommonPlan<VM>,
    //pub mark_count: Mutex<u8>
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
            //mark_count: Mutex::new(1)
        }
    }

    fn is_malloced(&self, object: ObjectReference) -> bool {
        NODES.lock().unwrap().contains(&object)
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
        println!("MarkSweep!!");
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
        self.common.prepare(tls, true);      
    }

    fn release(&self, tls: OpaquePointer) {
        //TODO: release dead objects
        
    }


    fn get_collection_reserve(&self) -> usize {
        0
        //self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        0
        //self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

// impl<VM: VMBinding> MarkSweep<VM> {
//     fn get_mark_count(&self) -> u8 {
//         *self.mark_count.lock().unwrap()
//     }

//     fn increment_mark_count(&self) {
//         let mark_count_u8 = self.get_mark_count();
//         self.mark_count.lock().unwrap().checked_add(1);
//     }
// }