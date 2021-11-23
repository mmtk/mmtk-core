use super::gc_work::SSGCWorkContext;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::semispace::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::copyspace::CopySpace;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::opaque_pointer::VMWorkerThread;
use crate::util::options::UnsafeOptionsWrapper;
use crate::{plan::global::BasePlan, vm::VMBinding};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use enum_map::EnumMap;

pub struct SemiSpace<VM: VMBinding> {
    pub hi: AtomicBool,
    pub copyspace0: CopySpace<VM>,
    pub copyspace1: CopySpace<VM>,
    pub common: CommonPlan<VM>,
}

pub const SS_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    max_non_los_default_alloc_bytes:
        crate::plan::plan_constraints::MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for SemiSpace<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &SS_CONSTRAINTS
    }

    fn create_worker_local(&'static self, tls: VMWorkerThread) -> GCWorkerLocalPtr {
        use enum_map::enum_map;
        use crate::util::copy::*;

        GCWorkerLocalPtr::new(GCWorkerCopyContext::new(tls, self, CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::CopySpace(0),
                _ => CopySelector::Unused,
            },
            constraints: &SS_CONSTRAINTS,
        }, &[
            // // The tospace argument doesn't matter, we will rebind before a GC anyway.
            (CopySelector::CopySpace(0), &self.copyspace0)
        ]))
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);

        self.copyspace0.init(vm_map);
        self.copyspace1.init(vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.schedule_common_work::<SSGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);

        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst); // flip the semi-spaces
                                                                       // prepare each of the collected regions
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
    }

    fn prepare_worker(&self, worker: &mut GCWorker<VM>) {
        // use crate::policy::copyspace::CopySpaceCopyContext;
        // let copy_context = unsafe { worker.local::<CopySpaceCopyContext<VM>>() };
        // copy_context.rebind(self.tospace());
        use crate::util::copy::*;

        let copy_context = unsafe { worker.local::<GCWorkerCopyContext<VM>>() };
        unsafe { copy_context.copy[0].assume_init_mut() }.rebind(self.tospace());
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.fromspace().release();
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
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
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);

        let res = SemiSpace {
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
                &SS_CONSTRAINTS,
                global_metadata_specs,
            ),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            res.common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace0
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            res.copyspace1
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        res
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
