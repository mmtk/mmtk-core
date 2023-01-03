use super::gc_work::GenImmixMatureGCWorkContext;
use super::gc_work::GenImmixNurseryGCWorkContext;
use crate::plan::generational::global::Gen;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::ImmixSpace;
use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST};
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::VMRequest;
use crate::util::VMWorkerThread;
use crate::vm::*;

use enum_map::EnumMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use mmtk_macros::PlanTraceObject;

/// Generational immix. This implements the functionality of a two-generation copying
/// collector where the higher generation is an immix space.
/// See the PLDI'08 paper by Blackburn and McKinley for a description
/// of the algorithm: <http://doi.acm.org/10.1145/1375581.1375586>.
#[derive(PlanTraceObject)]
pub struct GenImmix<VM: VMBinding> {
    /// Generational plan, which includes a nursery space and operations related with nursery.
    #[fallback_trace]
    pub gen: Gen<VM>,
    /// An immix space as the mature space.
    #[post_scan]
    #[trace(CopySemantics::Mature)]
    pub immix: ImmixSpace<VM>,
    /// Whether the last GC was a defrag GC for the immix space.
    pub last_gc_was_defrag: AtomicBool,
    /// Whether the last GC was a full heap GC
    pub last_gc_was_full_heap: AtomicBool,
}

pub const GENIMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    // The maximum object size that can be allocated without LOS is restricted by the max immix object size.
    // This might be too restrictive, as our default allocator is bump pointer (nursery allocator) which
    // can allocate objects larger than max immix object size. However, for copying, we haven't implemented
    // copying to LOS so we always copy from nursery to the mature immix space. In this case, we should not
    // allocate objects larger than the max immix object size to nursery as well.
    // TODO: We may want to fix this, as this possibly has negative performance impact.
    max_non_los_default_alloc_bytes: crate::util::rust_util::min_of_usize(
        crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
        crate::plan::generational::GEN_CONSTRAINTS.max_non_los_default_alloc_bytes,
    ),
    ..crate::plan::generational::GEN_CONSTRAINTS
};

impl<VM: VMBinding> Plan for GenImmix<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &GENIMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::PromoteToMature => CopySelector::Immix(0),
                CopySemantics::Mature => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix)],
            constraints: &GENIMMIX_CONSTRAINTS,
        }
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        self.last_gc_was_full_heap.load(Ordering::Relaxed)
            && ImmixSpace::<VM>::is_last_gc_exhaustive(
                self.last_gc_was_defrag.load(Ordering::Relaxed),
            )
    }

    fn force_full_heap_collection(&self) {
        self.gen.force_full_heap_collection()
    }

    fn last_collection_full_heap(&self) -> bool {
        self.gen.last_collection_full_heap()
    }

    fn collection_required(&self, space_full: bool, space: Option<&dyn Space<Self::VM>>) -> bool
    where
        Self: Sized,
    {
        self.gen.collection_required(self, space_full, space)
    }

    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>> {
        let mut ret = self.gen.get_spaces();
        ret.push(&self.immix);
        ret
    }

    // GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }> and GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>
    // are different types. However, it seems clippy does not recognize the constant type parameter and thinks we have identical blocks
    // in different if branches.
    #[allow(clippy::if_same_then_else)]
    #[allow(clippy::branches_sharing_code)]
    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<Self::VM>) {
        let is_full_heap = self.requires_full_heap_collection();

        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        let defrag = if is_full_heap {
            self.immix.decide_whether_to_defrag(
                self.is_emergency_collection(),
                true,
                self.base().cur_collection_attempts.load(Ordering::SeqCst),
                self.base().is_user_triggered_collection(),
                *self.base().options.full_heap_system_gc,
            )
        } else {
            false
        };

        if !is_full_heap {
            debug!("Nursery GC");
            scheduler.schedule_common_work::<GenImmixNurseryGCWorkContext<VM>>(self);
        } else if defrag {
            debug!("Full heap GC Defrag");
            scheduler
                .schedule_common_work::<GenImmixMatureGCWorkContext<VM, TRACE_KIND_DEFRAG>>(self);
        } else {
            debug!("Full heap GC Fast");
            scheduler
                .schedule_common_work::<GenImmixMatureGCWorkContext<VM, TRACE_KIND_FAST>>(self);
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*super::mutator::ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.gen.prepare(tls);
        if full_heap {
            self.immix.prepare(full_heap);
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.is_current_gc_nursery();
        self.gen.release(tls);
        if full_heap {
            let did_defrag = self.immix.release(full_heap);
            self.last_gc_was_defrag.store(did_defrag, Ordering::Relaxed);
        } else {
            self.last_gc_was_defrag.store(false, Ordering::Relaxed);
        }
        self.last_gc_was_full_heap
            .store(full_heap, Ordering::Relaxed);
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {
        self.gen
            .set_next_gc_full_heap(Gen::should_next_gc_be_full_heap(self));
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.gen.get_collection_reserved_pages() + self.immix.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.gen.get_used_pages() + self.immix.reserved_pages()
    }

    /// Return the number of pages available for allocation. Assuming all future allocations goes to nursery.
    fn get_available_pages(&self) -> usize {
        // super.get_available_pages() / 2 to reserve pages for copying
        (self
            .get_total_pages()
            .saturating_sub(self.get_reserved_pages()))
            >> 1
    }

    fn get_mature_physical_pages_available(&self) -> usize {
        self.immix.available_physical_pages()
    }

    fn get_mature_reserved_pages(&self) -> usize {
        self.immix.reserved_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.gen.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.gen.common
    }

    fn generational(&self) -> Option<&Gen<VM>> {
        Some(&self.gen)
    }

    fn is_current_gc_nursery(&self) -> bool {
        !self.gen.gc_full_heap.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> GenImmix<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &GENIMMIX_CONSTRAINTS,
            global_side_metadata_specs:
                crate::plan::generational::new_generational_global_metadata_specs::<VM>(),
        };
        let immix_space = ImmixSpace::new(plan_args.get_space_args(
            "immix_mature",
            true,
            VMRequest::discontiguous(),
        ));

        let genimmix = GenImmix {
            gen: Gen::new(plan_args),
            immix: immix_space,
            last_gc_was_defrag: AtomicBool::new(false),
            last_gc_was_full_heap: AtomicBool::new(false),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        {
            use crate::util::metadata::side_metadata::SideMetadataSanity;
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            genimmix
                .gen
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            genimmix
                .immix
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        genimmix
    }

    fn requires_full_heap_collection(&self) -> bool {
        self.gen.requires_full_heap_collection(self)
    }
}
