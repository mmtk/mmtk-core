use super::gc_works::{MSProcessEdges};
use crate::{mmtk::MMTK, plan::{self, global::NoCopy}, util::{Address, ObjectReference}};
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::marksweep::mutator::create_ms_mutator;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::scheduler::gc_works::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::sync::Arc;
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
        // Do nothing  
    }

    fn release(&self, tls: OpaquePointer) {
        println!("called marksweep release");
        unsafe {
        let mut NODES_hs = &mut *NODES.lock().unwrap();
        NODES_hs.retain(|&o| plan::marksweep::global::MarkSweep::<VM>::marked(&o));
        for object in NODES_hs.iter() {
            let a: Address = Address::from_usize(object.to_address().as_usize() - 8);
            let marking_word: usize = unsafe { a.load() };
            debug_assert!(marking_word != 0usize, "Marking word is 0, should have been removed from NODES");
            debug_assert!(marking_word == 1usize, "Marking word must be 1 or 0, found {}", marking_word);
            unsafe { a.store(0) };
        }
    }
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

impl<VM: VMBinding> MarkSweep<VM> {
    fn marked(&object: &ObjectReference) -> bool {
        unsafe {
            let a: Address = Address::from_usize(object.to_address().as_usize() - 8);
            let marking_word: usize = unsafe { a.load() };
            if marking_word == 0 {
                libc::free(a.to_mut_ptr());
                return false
            }
            true
        }
    }
}
