use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::marksweep::gc_work::{MSGCWorkContext, MSSweepChunks};
use crate::plan::marksweep::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::mallocspace::metadata::ACTIVE_CHUNK_METADATA_SPEC;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(not(feature = "global_alloc_bit"))]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use std::sync::Arc;

use enum_map::EnumMap;

pub struct MarkSweep<VM: VMBinding> {
    common: CommonPlan<VM>,
    ms: MallocSpace<VM>,
}

pub const MS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    may_trace_duplicate_edges: true,
    ..PlanConstraints::default()
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
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.schedule_common_work::<MSGCWorkContext<VM>>(self);
        scheduler.work_buckets[WorkBucketStage::Prepare].add(MSSweepChunks::<VM>::new(self));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        // Dont need to prepare for MallocSpace
    }

    fn release(&mut self, tls: VMWorkerThread) {
        trace!("Marksweep: Release");
        self.common.release(tls, true);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn get_collection_reserve(&self) -> usize {
        0
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
}

impl<VM: VMBinding> MarkSweep<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let heap = HeapMeta::new(HEAP_START, HEAP_END);
        // if global_alloc_bit is enabled, ALLOC_SIDE_METADATA_SPEC will be added to
        // SideMetadataContext by default, so we don't need to add it here.
        #[cfg(feature = "global_alloc_bit")]
        let global_metadata_specs =
            SideMetadataContext::new_global_specs(&[ACTIVE_CHUNK_METADATA_SPEC]);
        // if global_alloc_bit is NOT enabled,
        // we need to add ALLOC_SIDE_METADATA_SPEC to SideMetadataContext here.
        #[cfg(not(feature = "global_alloc_bit"))]
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[
            ALLOC_SIDE_METADATA_SPEC,
            ACTIVE_CHUNK_METADATA_SPEC,
        ]);

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

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            res.common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.ms
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        res
    }

    pub fn ms_space(&self) -> &MallocSpace<VM> {
        &self.ms
    }
}
