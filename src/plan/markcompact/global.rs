use super::gc_work::{CalcFwdAddr, Compact, ForwardingProcessEdges, MarkingProcessEdges};
use crate::mmtk::MMTK;
use crate::plan::global::BasePlan; //Modify
use crate::plan::global::CommonPlan; // Add
use crate::plan::global::GcStatus;
use crate::plan::global::NoCopy;
// Add
use crate::plan::markcompact::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*; // Add
use crate::scheduler::*; // Modify
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(not(feature = "global_alloc_bit"))]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::opaque_pointer::*;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;

// pub const ALLOC_MARKCOMPACT: AllocationSemantics = AllocationSemantics::Default; // Add

pub struct MarkCompact<VM: VMBinding> {
    pub mc_space: MarkCompactSpace<VM>,
    pub common: CommonPlan<VM>,
}

pub const MARKCOMPACT_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 1,
    gc_extra_header_words: 1,
    num_specialized_scans: 2,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkCompact<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &MARKCOMPACT_CONSTRAINTS
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

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.mc_space.init(&vm_map);
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn prepare(&mut self, _tls: VMWorkerThread) {
        self.common.prepare(_tls, true);
        self.mc_space.prepare();
    }

    fn release(&mut self, _tls: VMWorkerThread) {
        self.common.release(_tls, true);
        self.mc_space.release();
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MarkingProcessEdges<VM>>::new());
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, NoCopy<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::RefClosure]
            .add(ProcessWeakRefs::<MarkingProcessEdges<VM>>::new());

        scheduler.work_buckets[WorkBucketStage::CalculateForwarding]
            .add(CalcFwdAddr::<VM>::new(&self.mc_space));
        // do another trace to update references
        scheduler.work_buckets[WorkBucketStage::RefForwarding]
            .add(ScanStackRoots::<ForwardingProcessEdges<VM>>::new());
        scheduler.work_buckets[WorkBucketStage::RefForwarding]
            .add(ScanVMSpecificRoots::<ForwardingProcessEdges<VM>>::new());

        scheduler.work_buckets[WorkBucketStage::Compact].add(Compact::<VM>::new(&self.mc_space));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, NoCopy<VM>>::new(self));

        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final].add(
            crate::util::sanity::sanity_checker::ScheduleSanityGC::<Self, NoCopy<VM>>::new(self),
        );
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn get_pages_used(&self) -> usize {
        self.mc_space.reserved_pages() + self.common.get_pages_used()
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_extra_header_bytes(&self) -> usize {
        std::cmp::max(
            MARKCOMPACT_CONSTRAINTS.gc_extra_header_words * crate::util::constants::BYTES_IN_WORD,
            VM::VMObjectModel::object_alignment() as usize,
        )
        .next_power_of_two()
    }
}

impl<VM: VMBinding> MarkCompact<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        // if global_alloc_bit is enabled, ALLOC_SIDE_METADATA_SPEC will be added to
        // SideMetadataContext by default, so we don't need to add it here.
        #[cfg(feature = "global_alloc_bit")]
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        // if global_alloc_bit is NOT enabled,
        // we need to add ALLOC_SIDE_METADATA_SPEC to SideMetadataContext here.
        #[cfg(not(feature = "global_alloc_bit"))]
        let global_metadata_specs =
            SideMetadataContext::new_global_specs(&[ALLOC_SIDE_METADATA_SPEC]);

        let mc_space = MarkCompactSpace::new(
            "mark_compact_space",
            true,
            VMRequest::discontiguous(),
            global_metadata_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
        );

        let res = MarkCompact {
            mc_space,
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &MARKCOMPACT_CONSTRAINTS,
                global_metadata_specs.clone(),
            ),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.common
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.mc_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);

        res
    }
}

impl<VM: VMBinding> MarkCompact<VM> {
    pub fn mc_space(&self) -> &MarkCompactSpace<VM> {
        &self.mc_space
    }
}
