use super::gc_work::MarkCompactGCWorkContext;
use super::gc_work::{
    CalculateForwardingAddress, Compact, ForwardingProcessEdges, MarkingProcessEdges,
    UpdateReferences,
};
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::markcompact::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(not(feature = "global_alloc_bit"))]
use crate::util::alloc_bit::ALLOC_SIDE_METADATA_SPEC;
use crate::util::copy::CopySemantics;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::opaque_pointer::*;
use crate::util::options::Options;
use crate::vm::VMBinding;

use enum_map::EnumMap;
use std::sync::Arc;

use mmtk_macros::PlanTraceObject;

#[derive(PlanTraceObject)]
pub struct MarkCompact<VM: VMBinding> {
    #[trace(CopySemantics::DefaultCopy)]
    pub mc_space: MarkCompactSpace<VM>,
    #[fallback_trace]
    pub common: CommonPlan<VM>,
}

pub const MARKCOMPACT_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 1,
    num_specialized_scans: 2,
    needs_forward_after_liveness: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for MarkCompact<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &MARKCOMPACT_CONSTRAINTS
    }

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.common.get_spaces();
        ret.push(&self.mc_space);
        ret
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
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);

        // TODO use schedule_common once it can work with markcompact
        // self.common()
        //     .schedule_common::<Self, MarkingProcessEdges<VM>, NoCopy<VM>>(
        //         self,
        //         &MARKCOMPACT_CONSTRAINTS,
        //         scheduler,
        //     );

        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MarkingProcessEdges<VM>>::new());

        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<MarkCompactGCWorkContext<VM>>::new(self));

        scheduler.work_buckets[WorkBucketStage::CalculateForwarding]
            .add(CalculateForwardingAddress::<VM>::new(&self.mc_space));
        // do another trace to update references
        scheduler.work_buckets[WorkBucketStage::SecondRoots].add(UpdateReferences::<VM>::new());
        scheduler.work_buckets[WorkBucketStage::Compact].add(Compact::<VM>::new(&self.mc_space));

        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<MarkCompactGCWorkContext<VM>>::new(self));

        // Reference processing
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
            };
            scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<MarkingProcessEdges<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<MarkingProcessEdges<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<MarkingProcessEdges<VM>>::new());

            // VM-specific weak ref processing
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure].set_boss_work(Box::new(
                VMProcessWeakRefs::<MarkingProcessEdges<VM>>::new(false),
            ));

            use crate::util::reference_processor::RefForwarding;
            scheduler.work_buckets[WorkBucketStage::RefForwarding]
                .add(RefForwarding::<ForwardingProcessEdges<VM>>::new());

            // VM-specific weak ref forwarding
            scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_boss_work(Box::new(
                VMProcessWeakRefs::<ForwardingProcessEdges<VM>>::new(true),
            ));

            use crate::util::reference_processor::RefEnqueue;
            scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }

        // Finalization
        if !*self.base().options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            // treat finalizable objects as roots and perform a closure (marking)
            // must be done before calculating forwarding pointers
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<MarkingProcessEdges<VM>>::new());
            // update finalizable object references
            // must be done before compacting
            scheduler.work_buckets[WorkBucketStage::FinalizableForwarding]
                .add(ForwardFinalization::<ForwardingProcessEdges<VM>>::new());
        }

        // Analysis GC work
        #[cfg(feature = "analysis")]
        {
            use crate::util::analysis::GcHookWork;
            scheduler.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
        }
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(crate::util::sanity::sanity_checker::ScheduleSanityGC::<Self>::new(self));
    }

    fn collection_required(&self, space_full: bool, _space: Option<&dyn Space<Self::VM>>) -> bool {
        self.base().collection_required(self, space_full)
    }

    fn get_used_pages(&self) -> usize {
        self.mc_space.reserved_pages() + self.common.get_used_pages()
    }

    fn get_collection_reserved_pages(&self) -> usize {
        0
    }
}

impl<VM: VMBinding> MarkCompact<VM> {
    pub fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<Options>) -> Self {
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
                global_metadata_specs,
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
