use super::gc_works::{SSCopyContext, SSProcessEdges};
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::Mutator;
use crate::plan::semispace::mutator::create_ss_mutator;
use crate::plan::semispace::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::policy::copyspace::CopySpace;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::plan::global::PlanTypes;

use enum_map::EnumMap;

pub type SelectedPlan<VM> = SemiSpace<VM>;

pub const ALLOC_SS: AllocationSemantics = AllocationSemantics::Default;

pub struct SemiSpace<VM: VMBinding> {
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
}

unsafe impl<VM: VMBinding> Sync for SemiSpace<VM> {}

impl<VM: VMBinding> PlanTypes for SemiSpace<VM> {
    type VM = VM;
    type Mutator = Mutator<Self>;
    type CopyContext = SSCopyContext<VM>;

    fn bind_mutator(
        &'static self,
        tls: OpaquePointer,
        _mmtk: &'static MMTK<Self::VM>,
    ) -> Box<Mutator<Self>> {
        Box::new(create_ss_mutator(tls, self))
    }    
}

impl<VM: VMBinding> Plan for SemiSpace<VM> {
    type VM = VM;

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);

        self.copyspace0.init(&vm_map);
        self.copyspace1.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler
            .unconstrained_works
            .add(StopMutators::<SSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.prepare_stage.add(Prepare::new(self));
        // Release global/collectors/mutators
        scheduler.release_stage.add(Release::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.final_stage.add(ScheduleSanityGC);
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
                                                                       // prepare each of the collected regions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        // release the collected region
        self.fromspace().release();
    }

    fn get_collection_reserve(&self) -> usize {
        self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.tospace().reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> SemiSpace<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        _scheduler: &'static MMTkScheduler<VM>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        SemiSpace {
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }
}
