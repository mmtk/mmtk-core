use crate::mmtk::MMTK;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::{BasePlan, CommonPlan, NoCopy};
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::mallocspace::metadata::ACTIVE_CHUNK_METADATA_SPEC;
use crate::plan::{AllocationSemantics, GcStatus};
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::{CommonSpace, Space, SpaceOptions};
use crate::scheduler::gc_work::{EndOfGC, Prepare, Release, StopMutators, ProcessWeakRefs};
use crate::scheduler::{GCWorkScheduler, GCWorkerLocalPtr};
use crate::scheduler::{GCWorkerLocal, WorkBucketStage};
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::free_list_allocator::BYTES_IN_BLOCK;
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{
    SideMetadataContext, SideMetadataSanity, SideMetadataSpec, LOCAL_SIDE_METADATA_BASE_ADDRESS,
};
use crate::util::opaque_pointer::*;
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::ScheduleSanityGC;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;
use crate::Mutator;

pub struct MarkSweep<VM: VMBinding> {
    common: CommonPlan<VM>,
    #[cfg(feature="malloc")]
    ms: MallocSpace<VM>,
    #[cfg(not(feature="malloc"))]
    ms: MarkSweepSpace<VM>,
}

pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    max_non_los_default_alloc_bytes: BYTES_IN_BLOCK,
    max_non_los_copy_bytes: BYTES_IN_BLOCK,
    needs_linear_scan: crate::util::constants::SUPPORT_CARD_SCANNING || crate::util::constants::LAZY_SWEEP,
    needs_concurrent_workers: false,
    generate_gc_trace: false,
    may_trace_duplicate_edges: true,
    needs_forward_after_liveness: false,
    needs_log_bit: false,
    barrier: crate::BarrierSelector::NoBarrier,
};

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);

        // FIXME correctly initialize spaces based on options
        self.ms.init(&vm_map);
    }





    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MSProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, NoCopy<VM>>::new(self));
            #[cfg(feature="malloc")]
            {
            scheduler.work_buckets[WorkBucketStage::Prepare].add(MSSweepChunks::<VM>::new(self));
            scheduler.work_buckets[WorkBucketStage::RefClosure]
                .add(ProcessWeakRefs::<MSProcessEdges<VM>>::new());
            }
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, NoCopy<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, NoCopy<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        #[cfg(not(feature="malloc"))]
        self.ms.reset();
    }

    fn release(&mut self, tls: VMWorkerThread) {
        #[cfg(not(any(feature="malloc", feature="eager_sweeping")))]
        self.ms.block_level_sweep();
        self.common.release(tls, true);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }


    fn get_pages_used(&self) -> usize {
        self.common.get_pages_used() + self.ms.reserved_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &MS_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: VMWorkerThread,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = NoCopy::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    #[cfg(not(feature="malloc"))]
    fn destroy_mutator(&self, mutator: &mut Mutator<VM>) {
        unsafe { 
            mutator.allocators.free_list[0].assume_init_mut().abandon_blocks();
        }
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        // if global_alloc_bit is enabled, ALLOC_SIDE_METADATA_SPEC will be added to
        // SideMetadataContext by default, so we don't need to add it here.
        #[cfg(feature = "global_alloc_bit")]
        let global_metadata_specs =
            SideMetadataContext::new_global_specs(&[]);
        // if global_alloc_bit is NOT enabled,
        // we need to add ALLOC_SIDE_METADATA_SPEC to SideMetadataContext here.
        #[cfg(not(feature = "global_alloc_bit"))]
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[
            ALLOC_SIDE_METADATA_SPEC,
        ]);
        
        #[cfg(not(feature = "malloc"))]
        let res = {
            let ms = MarkSweepSpace::new(
                "MSspace",
                true,
                VMRequest::discontiguous(),
                // local_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
                scheduler,
            );

            let common =             CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &MS_CONSTRAINTS,
                global_metadata_specs,
            );

            MarkSweep {
                common,
                ms,
            }
        };

        #[cfg(feature = "malloc")]
        let res = MarkSweep {
            ms: MallocSpace::new(global_metadata_specs.clone()),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &MS_CONSTRAINTS,
                global_metadata_specs,
            ),
        };

        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.common
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.ms
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res
    }

    #[cfg(feature="malloc")]
    pub fn ms_space(&self) -> &MallocSpace<VM> {
        &self.ms
    }

    #[cfg(not(feature="malloc"))]
    pub fn ms_space(&self) -> &MarkSweepSpace<VM> {
        &self.ms
    }

    /// Verify side metadata specs used in the spaces in Gen.
    pub fn verify_side_metadata_sanity(&self, sanity: &mut SideMetadataSanity) {
        self.ms.verify_side_metadata_sanity(sanity);
        self.common.verify_side_metadata_sanity(sanity);
    }
}