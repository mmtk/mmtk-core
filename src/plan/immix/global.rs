use super::gc_work::ImmixGCWorkContext;
use super::gc_work::TraceKind;
use super::mutator::ALLOCATOR_MAPPING;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::GcStatus;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use crate::util::copy::GCWorkerCopyContext;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use atomic::Ordering;
use enum_map::EnumMap;

pub struct Immix<VM: VMBinding> {
    pub immix_space: ImmixSpace<VM>,
    pub common: CommonPlan<VM>,
    last_gc_was_defrag: AtomicBool,
}

pub const IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: true,
    gc_header_bits: 2,
    gc_header_words: 0,
    num_specialized_scans: 1,
    /// Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for Immix<VM> {
    type VM = VM;

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        self.last_gc_was_defrag.load(Ordering::Relaxed)
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &IMMIX_CONSTRAINTS
    }

    fn create_worker_local(&'static self, tls: VMWorkerThread) -> GCWorkerCopyContext<VM> {
        use enum_map::enum_map;
        use crate::util::copy::*;

        GCWorkerCopyContext::new(tls, self, CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCompact => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            constraints: &IMMIX_CONSTRAINTS,
        }, &[
            (CopySelector::Immix(0), &self.immix_space),
        ])
    }

    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<GCWorkScheduler<VM>>,
    ) {
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.immix_space.init(vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        let in_defrag = self.immix_space.decide_whether_to_defrag(
            self.is_emergency_collection(),
            true,
            self.base().cur_collection_attempts.load(Ordering::SeqCst),
            self.base().is_user_triggered_collection(),
            self.base().options.full_heap_system_gc,
        );

        // The blocks are not identical, clippy is wrong. Probably it does not recognize the constant type parameter.
        #[allow(clippy::if_same_then_else)]
        if in_defrag {
            scheduler.schedule_common_work::<ImmixGCWorkContext<VM, { TraceKind::Defrag }>>(self);
        } else {
            scheduler.schedule_common_work::<ImmixGCWorkContext<VM, { TraceKind::Fast }>>(self);
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.immix_space.prepare(true);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        // release the collected region
        self.last_gc_was_defrag
            .store(self.immix_space.release(true), Ordering::Relaxed);
    }

    fn get_collection_reserve(&self) -> usize {
        self.immix_space.defrag_headroom_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.immix_space.reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> Immix<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        scheduler: Arc<GCWorkScheduler<VM>>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);
        let immix = Immix {
            immix_space: ImmixSpace::new(
                "immix",
                vm_map,
                mmapper,
                &mut heap,
                scheduler,
                global_metadata_specs.clone(),
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &IMMIX_CONSTRAINTS,
                global_metadata_specs,
            ),
            last_gc_was_defrag: AtomicBool::new(false),
        };

        {
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            immix
                .common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            immix
                .immix_space
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        immix
    }
}
