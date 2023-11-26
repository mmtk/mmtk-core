use super::gc_work::GenImmixMatureGCWorkContext;
use super::gc_work::GenImmixNurseryGCWorkContext;
use crate::plan::generational::global::CommonGenPlan;
use crate::plan::generational::global::GenerationalPlan;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::ImmixSpace;
use crate::policy::immix::ImmixSpaceArgs;
use crate::policy::immix::{TRACE_KIND_DEFRAG, TRACE_KIND_FAST};
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::scheduler::GCWorker;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::VMRequest;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::VMWorkerThread;
use crate::vm::*;
use crate::ObjectQueue;

use enum_map::EnumMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use mmtk_macros::{HasSpaces, PlanTraceObject};

/// Generational immix. This implements the functionality of a two-generation copying
/// collector where the higher generation is an immix space.
/// See the PLDI'08 paper by Blackburn and McKinley for a description
/// of the algorithm: <http://doi.acm.org/10.1145/1375581.1375586>.
#[derive(HasSpaces, PlanTraceObject)]
pub struct GenImmix<VM: VMBinding> {
    /// Generational plan, which includes a nursery space and operations related with nursery.
    #[parent]
    pub gen: CommonGenPlan<VM>,
    /// An immix space as the mature space.
    #[post_scan]
    #[space]
    #[copy_semantics(CopySemantics::Mature)]
    pub immix_space: ImmixSpace<VM>,
    /// Whether the last GC was a defrag GC for the immix space.
    pub last_gc_was_defrag: AtomicBool,
    /// Whether the last GC was a full heap GC
    pub last_gc_was_full_heap: AtomicBool,
}

/// The plan constraints for the generational immix plan.
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
    fn constraints(&self) -> &'static PlanConstraints {
        &GENIMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::PromoteToMature => CopySelector::ImmixHybrid(0),
                CopySemantics::Mature => CopySelector::ImmixHybrid(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::ImmixHybrid(0), &self.immix_space)],
            constraints: &GENIMMIX_CONSTRAINTS,
        }
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        self.last_gc_was_full_heap.load(Ordering::Relaxed)
            && ImmixSpace::<VM>::is_last_gc_exhaustive(
                self.last_gc_was_defrag.load(Ordering::Relaxed),
            )
    }

    fn collection_required(&self, space_full: bool, space: Option<&dyn Space<Self::VM>>) -> bool
    where
        Self: Sized,
    {
        self.gen.collection_required(self, space_full, space)
    }

    // GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }> and GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>
    // are different types. However, it seems clippy does not recognize the constant type parameter and thinks we have identical blocks
    // in different if branches.
    #[allow(clippy::if_same_then_else)]
    #[allow(clippy::branches_sharing_code)]
    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<Self::VM>) {
        let is_full_heap = self.requires_full_heap_collection();
        if !is_full_heap {
            debug!("Nursery GC");
            scheduler.schedule_common_work::<GenImmixNurseryGCWorkContext<VM>>(self);
        } else {
            crate::plan::immix::Immix::schedule_immix_full_heap_collection::<
                GenImmix<VM>,
                GenImmixMatureGCWorkContext<VM, TRACE_KIND_FAST>,
                GenImmixMatureGCWorkContext<VM, TRACE_KIND_DEFRAG>,
            >(self, &self.immix_space, scheduler);
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &super::mutator::ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.gen.is_current_gc_nursery();
        self.gen.prepare(tls);
        if full_heap {
            self.immix_space.prepare(
                full_heap,
                crate::policy::immix::defrag::StatsForDefrag::new(self),
            );
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let full_heap = !self.gen.is_current_gc_nursery();
        self.gen.release(tls);
        if full_heap {
            let did_defrag = self.immix_space.release(full_heap);
            self.last_gc_was_defrag.store(did_defrag, Ordering::Relaxed);
        } else {
            self.last_gc_was_defrag.store(false, Ordering::Relaxed);
        }
        self.last_gc_was_full_heap
            .store(full_heap, Ordering::Relaxed);
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {
        self.gen
            .set_next_gc_full_heap(CommonGenPlan::should_next_gc_be_full_heap(self));
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.gen.get_collection_reserved_pages() + self.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.gen.get_used_pages() + self.immix_space.reserved_pages()
    }

    /// Return the number of pages available for allocation. Assuming all future allocations goes to nursery.
    fn get_available_pages(&self) -> usize {
        // super.get_available_pages() / 2 to reserve pages for copying
        (self
            .get_total_pages()
            .saturating_sub(self.get_reserved_pages()))
            >> 1
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.gen.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.gen.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.gen.common
    }

    fn generational(&self) -> Option<&dyn GenerationalPlan<VM = VM>> {
        Some(self)
    }
}

impl<VM: VMBinding> GenerationalPlan for GenImmix<VM> {
    fn is_current_gc_nursery(&self) -> bool {
        self.gen.is_current_gc_nursery()
    }

    fn is_object_in_nursery(&self, object: ObjectReference) -> bool {
        self.gen.nursery.in_space(object)
    }

    fn is_address_in_nursery(&self, addr: Address) -> bool {
        self.gen.nursery.address_in_space(addr)
    }

    fn get_mature_physical_pages_available(&self) -> usize {
        self.immix_space.available_physical_pages()
    }

    fn get_mature_reserved_pages(&self) -> usize {
        self.immix_space.reserved_pages()
    }

    fn force_full_heap_collection(&self) {
        self.gen.force_full_heap_collection()
    }

    fn last_collection_full_heap(&self) -> bool {
        self.gen.last_collection_full_heap()
    }
}

impl<VM: VMBinding> crate::plan::generational::global::GenerationalPlanExt<VM> for GenImmix<VM> {
    fn trace_object_nursery<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        self.gen.trace_object_nursery(queue, object, worker)
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
        let immix_space = ImmixSpace::new(
            plan_args.get_space_args("immix_mature", true, VMRequest::discontiguous()),
            ImmixSpaceArgs {
                reset_log_bit_in_major_gc: false,
                // We don't need to unlog objects at tracing. Instead, we unlog objects at copying.
                // Any object is moved into the mature space, or is copied inside the mature space. We will unlog it.
                unlog_object_when_traced: false,
                // In GenImmix, young objects are not allocated in ImmixSpace directly.
                mixed_age: false,
            },
        );

        let genimmix = GenImmix {
            gen: CommonGenPlan::new(plan_args),
            immix_space,
            last_gc_was_defrag: AtomicBool::new(false),
            last_gc_was_full_heap: AtomicBool::new(false),
        };

        genimmix.verify_side_metadata_sanity();

        genimmix
    }

    fn requires_full_heap_collection(&self) -> bool {
        self.gen.requires_full_heap_collection(self)
    }
}
