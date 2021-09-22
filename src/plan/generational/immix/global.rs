use super::gc_work::{GenImmixCopyContext, GenImmixMatureProcessEdges};
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::generational::global::Gen;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::immix::gc_work::TraceKind;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::ImmixSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkScheduler;
use crate::scheduler::GCWorkerLocalPtr;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::options::UnsafeOptionsWrapper;
use crate::util::VMWorkerThread;
use crate::vm::*;
use crate::MMTK;

use enum_map::EnumMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Generational immix. This implements the functionality of a two-generation copying
/// collector where the higher generation is an immix space.
/// See the PLDI'08 paper by Blackburn and McKinley for a description
/// of the algorithm: http://doi.acm.org/10.1145/1375581.137558.
pub struct GenImmix<VM: VMBinding> {
    /// Generational plan, which includes a nursery space and operations related with nursery.
    pub gen: Gen<VM>,
    /// An immix space as the mature space.
    pub immix: ImmixSpace<VM>,
    /// Whether the last GC was a defrag GC for the immix space.
    // This is not used. It should be used for last_collection_was_exhaustive.
    // TODO: We need to fix this.
    pub last_gc_was_defrag: AtomicBool,
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

    fn create_worker_local(
        &self,
        tls: VMWorkerThread,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = GenImmixCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        self.gen.collection_required(self, space_full, space)
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.gen.gc_init(heap_size, vm_map, scheduler);
        self.immix.init(vm_map);
    }

    // GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }> and GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>
    // are different types. However, it seems clippy does not recognize the constant type parameter and thinks we have identical blocks
    // in different if branches.
    #[allow(clippy::if_same_then_else)]
    #[allow(clippy::branches_sharing_code)]
    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<Self::VM>) {
        let is_full_heap = self.request_full_heap_collection();

        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        let defrag = if is_full_heap {
            self.immix.decide_whether_to_defrag(
                self.is_emergency_collection(),
                true,
                self.base().cur_collection_attempts.load(Ordering::SeqCst),
                self.base().is_user_triggered_collection(),
                self.base().options.full_heap_system_gc,
            )
        } else {
            false
        };

        if !is_full_heap {
            debug!("Nursery GC");
            self.common()
                .schedule_common::<GenNurseryProcessEdges<VM, GenImmixCopyContext<VM>>>(
                    &GENIMMIX_CONSTRAINTS,
                    scheduler,
                );
            // Stop & scan mutators (mutator scanning can happen before STW)
            scheduler.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<
                GenNurseryProcessEdges<VM, GenImmixCopyContext<VM>>,
            >::new());
        } else if defrag {
            debug!("Full heap GC Defrag");
            self.common()
                .schedule_common::<GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }>>(
                    &GENIMMIX_CONSTRAINTS,
                    scheduler,
                );
            // Stop & scan mutators (mutator scanning can happen before STW)
            scheduler.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<
                GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }>,
            >::new());
        } else {
            debug!("Full heap GC Fast");
            self.common()
                .schedule_common::<GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>>(
                    &GENIMMIX_CONSTRAINTS,
                    scheduler,
                );
            // Stop & scan mutators (mutator scanning can happen before STW)
            scheduler.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<
                GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>,
            >::new());
        }

        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, GenImmixCopyContext<VM>>::new(self));
        if is_full_heap {
            if defrag {
                scheduler.work_buckets[WorkBucketStage::RefClosure].add(ProcessWeakRefs::<
                    GenImmixMatureProcessEdges<VM, { TraceKind::Defrag }>,
                >::new());
            } else {
                scheduler.work_buckets[WorkBucketStage::RefClosure].add(ProcessWeakRefs::<
                    GenImmixMatureProcessEdges<VM, { TraceKind::Fast }>,
                >::new());
            }
        } else {
            scheduler.work_buckets[WorkBucketStage::RefClosure].add(ProcessWeakRefs::<
                GenNurseryProcessEdges<VM, GenImmixCopyContext<VM>>,
            >::new());
        }
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, GenImmixCopyContext<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        {
            use crate::util::sanity::sanity_checker::*;
            scheduler.work_buckets[WorkBucketStage::Final]
                .add(ScheduleSanityGC::<Self, GenImmixCopyContext<VM>>::new(self));
        }
        scheduler.set_finalizer(Some(EndOfGC));
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
    }

    fn get_collection_reserve(&self) -> usize {
        self.gen.get_collection_reserve() + self.immix.defrag_headroom_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.gen.get_pages_used() + self.immix.reserved_pages()
    }

    /// Return the number of pages avilable for allocation. Assuming all future allocations goes to nursery.
    fn get_pages_avail(&self) -> usize {
        // super.get_pages_avail() / 2 to reserve pages for copying
        (self.get_total_pages() - self.get_pages_reserved()) >> 1
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.gen.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.gen.common
    }

    fn generational(&self) -> &Gen<VM> {
        &self.gen
    }

    fn is_current_gc_nursery(&self) -> bool {
        !self.gen.gc_full_heap.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> GenImmix<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        // We have no specific side metadata for copying. So just use the ones from generational.
        let global_metadata_specs =
            crate::plan::generational::new_generational_global_metadata_specs::<VM>();
        let immix_space = ImmixSpace::new(
            "immix_mature",
            vm_map,
            mmapper,
            &mut heap,
            scheduler,
            global_metadata_specs.clone(),
        );

        let genimmix = GenImmix {
            gen: Gen::new(
                heap,
                global_metadata_specs,
                &GENIMMIX_CONSTRAINTS,
                vm_map,
                mmapper,
                options,
            ),
            immix: immix_space,
            last_gc_was_defrag: AtomicBool::new(false),
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

    fn request_full_heap_collection(&self) -> bool {
        self.gen
            .request_full_heap_collection(self.get_total_pages(), self.get_pages_reserved())
    }
}
