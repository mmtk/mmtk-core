use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::marksweep::gc_work::MSGCWorkContext;
#[cfg(feature = "malloc")]
use crate::plan::marksweep::gc_work::MSSweepChunks;
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
#[cfg(feature = "malloc")]
use crate::policy::mallocspace::MallocSpace;
#[cfg(not(feature = "malloc"))]
use crate::policy::marksweepspace::block::Block;
#[cfg(not(feature = "malloc"))]
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
#[cfg(feature = "malloc")]
use crate::scheduler::WorkBucketStage;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(not(feature = "global_alloc_bit"))]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
#[cfg(feature = "malloc")]
use crate::util::constants::MAX_INT;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
#[cfg(not(feature = "malloc"))]
use crate::util::linear_scan::Region;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::options::Options;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
#[cfg(not(feature = "malloc"))]
use crate::Mutator;
use enum_map::EnumMap;
use mmtk_macros::PlanTraceObject;
use std::sync::Arc;

#[derive(PlanTraceObject)]
pub struct MarkSweep<VM: VMBinding> {
    #[fallback_trace]
    common: CommonPlan<VM>,
    #[cfg(feature = "malloc")]
    #[trace]
    ms: MallocSpace<VM>,
    #[cfg(not(feature = "malloc"))]
    #[trace]
    ms: MarkSweepSpace<VM>,
}

pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    #[cfg(feature = "malloc")]
    max_non_los_default_alloc_bytes: MAX_INT,
    #[cfg(feature = "malloc")]
    max_non_los_copy_bytes: MAX_INT,
    #[cfg(not(feature = "malloc"))]
    max_non_los_default_alloc_bytes: Block::BYTES,
    #[cfg(not(feature = "malloc"))]
    max_non_los_copy_bytes: Block::BYTES,
    needs_linear_scan: crate::util::constants::SUPPORT_CARD_SCANNING
        || crate::util::constants::LAZY_SWEEP,
    needs_concurrent_workers: false,
    generate_gc_trace: false,
    may_trace_duplicate_edges: true,
    needs_forward_after_liveness: false,
    needs_log_bit: false,
    barrier: crate::BarrierSelector::NoBarrier,
};

impl<VM: VMBinding> Plan for MarkSweep<VM> {
    type VM = VM;

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.common.get_spaces();
        ret.push(&self.ms);
        ret
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.schedule_common_work::<MSGCWorkContext<VM>>(self);
        #[cfg(feature = "malloc")]
        scheduler.work_buckets[WorkBucketStage::Prepare].add(MSSweepChunks::<VM>::new(self));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        #[cfg(not(feature = "malloc"))]
        self.ms.reset();
    }

    fn release(&mut self, tls: VMWorkerThread) {
        #[cfg(not(any(feature = "malloc", feature = "eager_sweeping")))]
        self.ms.block_level_sweep();
        self.common.release(tls, true);
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_used_pages(&self) -> usize {
        self.common.get_used_pages() + self.ms.reserved_pages()
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

    #[cfg(not(feature = "malloc"))]
    fn destroy_mutator(&self, mutator: &mut Mutator<VM>) {
        unsafe {
            mutator.allocators.free_list[0]
                .assume_init_mut()
                .abandon_blocks();
        }
    }
}

impl<VM: VMBinding> MarkSweep<VM> {
    #[allow(unused_variables)] // scheduler only used by marksweepspace
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<Options>,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> Self {
        #[allow(unused_mut)]
        let mut heap = HeapMeta::new(&options);
        // if global_alloc_bit is enabled, ALLOC_SIDE_METADATA_SPEC will be added to
        // SideMetadataContext by default, so we don't need to add it here.
        #[cfg(feature = "global_alloc_bit")]
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        // if global_alloc_bit is NOT enabled,
        // we need to add ALLOC_SIDE_METADATA_SPEC to SideMetadataContext here.
        #[cfg(not(feature = "global_alloc_bit"))]
        let global_metadata_specs =
            SideMetadataContext::new_global_specs(&[ALLOC_SIDE_METADATA_SPEC]);

        #[cfg(not(feature = "malloc"))]
        let res = {
            let ms = MarkSweepSpace::new(
                "MarkSweepSpace",
                false,
                VMRequest::discontiguous(),
                // local_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
                scheduler,
            );

            let common = CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &MS_CONSTRAINTS,
                global_metadata_specs,
            );

            MarkSweep { common, ms }
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

    #[cfg(feature = "malloc")]
    pub fn ms_space(&self) -> &MallocSpace<VM> {
        &self.ms
    }

    #[cfg(not(feature = "malloc"))]
    pub fn ms_space(&self) -> &MarkSweepSpace<VM> {
        &self.ms
    }
}
