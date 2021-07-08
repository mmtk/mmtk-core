use super::gc_work::{GenCopyCopyContext, GenCopyMatureProcessEdges, GenCopyNurseryProcessEdges};
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::options::UnsafeOptionsWrapper;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::VMWorkerThread;
use crate::util::{conversions, metadata};
use crate::vm::*;
use crate::{mmtk::MMTK, plan::barriers::BarrierSelector};
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const ALLOC_SS: AllocationSemantics = AllocationSemantics::Default;

pub struct GenCopy<VM: VMBinding> {
    pub nursery: CopySpace<VM>,
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
    // TODO: These should belong to a common generational implementation.
    /// Is this GC full heap?
    gc_full_heap: AtomicBool,
    /// Is next GC full heap?
    next_gc_full_heap: AtomicBool,
}

pub const GENCOPY_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    barrier: super::ACTIVE_BARRIER,
    max_non_los_default_alloc_bytes: crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for GenCopy<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &GENCOPY_CONSTRAINTS
    }

    fn create_worker_local(
        &self,
        tls: VMWorkerThread,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = GenCopyCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool
    where
        Self: Sized,
    {
        let nursery_full = self.nursery.reserved_pages()
            >= (conversions::bytes_to_pages_up(self.base().options.max_nursery));
        if nursery_full {
            return true;
        }

        if space_full && space.common().descriptor != self.nursery.common().descriptor {
            self.next_gc_full_heap.store(true, Ordering::SeqCst);
        }

        self.base().collection_required(self, space_full, space)
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.nursery.init(&vm_map);
        self.copyspace0.init(&vm_map);
        self.copyspace1.init(&vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &MMTkScheduler<VM>) {
        let is_full_heap = self.request_full_heap_collection();
        self.gc_full_heap.store(is_full_heap, Ordering::SeqCst);

        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        if !is_full_heap {
            debug!("Nursery GC");
            self.common()
                .schedule_common::<GenCopyNurseryProcessEdges<VM>>(&GENCOPY_CONSTRAINTS, scheduler);
            // Stop & scan mutators (mutator scanning can happen before STW)
            scheduler.work_buckets[WorkBucketStage::Unconstrained]
                .add(StopMutators::<GenCopyNurseryProcessEdges<VM>>::new());
        } else {
            debug!("Full heap GC");
            self.common()
                .schedule_common::<GenCopyMatureProcessEdges<VM>>(&GENCOPY_CONSTRAINTS, scheduler);
            // Stop & scan mutators (mutator scanning can happen before STW)
            scheduler.work_buckets[WorkBucketStage::Unconstrained]
                .add(StopMutators::<GenCopyMatureProcessEdges<VM>>::new());
        }

        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<Self, GenCopyCopyContext<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<Self, GenCopyCopyContext<VM>>::new(self));
        // Resume mutators
        #[cfg(feature = "sanity")]
        scheduler.work_buckets[WorkBucketStage::Final]
            .add(ScheduleSanityGC::<Self, GenCopyCopyContext<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.nursery.prepare(true);
        if !self.is_current_gc_nursery() {
            self.hi
                .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
        }
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.nursery.release();
        if !self.is_current_gc_nursery() {
            self.fromspace().release();
        }

        self.next_gc_full_heap.store(
            self.get_pages_avail()
                < conversions::bytes_to_pages_up(self.base().options.min_nursery),
            Ordering::SeqCst,
        );
    }

    fn get_collection_reserve(&self) -> usize {
        self.nursery.reserved_pages() + self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.nursery.reserved_pages()
            + self.tospace().reserved_pages()
            + self.common.get_pages_used()
    }

    /// Return the number of pages avilable for allocation. Assuming all future allocations goes to nursery.
    fn get_pages_avail(&self) -> usize {
        // super.get_pages_avail() / 2 to reserve pages for copying
        (self.get_total_pages() - self.get_pages_reserved()) >> 1
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn is_current_gc_nursery(&self) -> bool {
        !self.gc_full_heap.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> GenCopy<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let gencopy_specs = if super::ACTIVE_BARRIER == BarrierSelector::ObjectBarrier {
            metadata::extract_side_metadata(&[VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC])
        } else {
            vec![]
        };
        let global_metadata_specs = SideMetadataContext::new_global_specs(&gencopy_specs);

        let res = GenCopy {
            nursery: CopySpace::new(
                "nursery",
                false,
                true,
                VMRequest::fixed_extent(crate::util::options::NURSERY_SIZE, false),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            hi: AtomicBool::new(false),
            copyspace0: CopySpace::new(
                "copyspace0",
                false,
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            copyspace1: CopySpace::new(
                "copyspace1",
                true,
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &GENCOPY_CONSTRAINTS,
                global_metadata_specs,
            ),
            gc_full_heap: AtomicBool::default(),
            next_gc_full_heap: AtomicBool::new(false),
        };

        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            res.common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.nursery
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace0
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace1
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        res
    }

    fn request_full_heap_collection(&self) -> bool {
        // For barrier overhead measurements, we always do full gc in nursery collections.
        if super::FULL_NURSERY_GC {
            return true;
        }

        if self.base().user_triggered_collection.load(Ordering::SeqCst)
            && self.base().options.full_heap_system_gc
        {
            return true;
        }

        if self.next_gc_full_heap.load(Ordering::SeqCst)
            || self.base().cur_collection_attempts.load(Ordering::SeqCst) > 1
        {
            // Forces full heap collection
            return true;
        }

        self.get_total_pages() <= self.get_pages_reserved()
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
