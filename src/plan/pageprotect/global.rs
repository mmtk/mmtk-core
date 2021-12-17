use super::gc_work::PPGCWorkContext;
use super::mutator::ALLOCATOR_MAPPING;
use crate::mmtk::MMTK;
use crate::plan::global::GcStatus;
use crate::plan::mutator_context::MutatorConfig;
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
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::options::UnsafeOptionsWrapper;
use crate::{plan::global::BasePlan, vm::VMBinding};
use crate::{
    plan::global::{CommonPlan, NoCopy},
    policy::largeobjectspace::LargeObjectSpace,
    util::opaque_pointer::VMWorkerThread,
};

use enum_map::EnumMap;
use std::sync::Arc;

pub struct PageProtect<VM: VMBinding> {
    pub space: LargeObjectSpace<VM>,
    pub common: CommonPlan<VM>,
}

pub const CONSTRAINTS: PlanConstraints = PlanConstraints {
    moves_objects: false,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for PageProtect<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &CONSTRAINTS
    }

    fn create_mutator_config(&'static self) -> MutatorConfig<VM> {
        use super::mutator::*;
        use crate::plan::mutator_context::create_space_mapping;
        MutatorConfig {
            allocator_mapping: &*ALLOCATOR_MAPPING,
            space_mapping: box {
                let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, self);
                vec.push((AllocatorSelector::LargeObject(0), &self.space));
                vec
            },
            prepare_func: &pp_mutator_prepare,
            release_func: &pp_mutator_release,
        }
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
        // Warn users that the plan may fail due to maximum mapping allowed.
        warn!(
            "PageProtect uses a high volume of memory mappings. \
            If you encounter failures in memory protect/unprotect in this plan,\
            consider increase the maximum mapping allowed by the OS{}.",
            if cfg!(target_os = "linux") {
                " (e.g. sudo sysctl -w vm.max_map_count=655300)"
            } else {
                ""
            }
        );
        self.common.gc_init(heap_size, vm_map, scheduler);
        self.space.init(vm_map);
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.base().set_collection_kind::<Self>(self);
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.schedule_common_work::<PPGCWorkContext<VM>>(self);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        self.common.prepare(tls, true);
        self.space.prepare(true);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.common.release(tls, true);
        self.space.release(true);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_pages_used(&self) -> usize {
        self.space.reserved_pages() + self.common.get_pages_used()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }
}

impl<VM: VMBinding> PageProtect<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        let global_metadata_specs = SideMetadataContext::new_global_specs(&[]);

        let ret = PageProtect {
            space: LargeObjectSpace::new(
                "los",
                true,
                VMRequest::discontiguous(),
                global_metadata_specs.clone(),
                vm_map,
                mmapper,
                &mut heap,
                &CONSTRAINTS,
                true,
            ),
            common: CommonPlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &CONSTRAINTS,
                global_metadata_specs,
            ),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        {
            use crate::util::metadata::side_metadata::SideMetadataSanity;
            let mut side_metadata_sanity_checker = SideMetadataSanity::new();
            ret.common
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
            ret.space
                .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        }

        ret
    }
}
