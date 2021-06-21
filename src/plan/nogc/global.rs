use crate::plan::global::BasePlan;
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::marksweepspace::MarkSweepSpace;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSanity};
use crate::util::opaque_pointer::*;
use crate::util::options::UnsafeOptionsWrapper;
use crate::vm::VMBinding;
use enum_map::EnumMap;
use std::sync::Arc;

#[cfg(not(feature = "nogc_lock_free"))]
use crate::policy::immortalspace::ImmortalSpace as NoGCImmortalSpace;
#[cfg(feature = "nogc_lock_free")]
use crate::policy::lockfreeimmortalspace::LockFreeImmortalSpace as NoGCImmortalSpace;

pub struct NoGC<VM: VMBinding> {
    pub base: BasePlan<VM>,
    pub nogc_space: MarkSweepSpace<VM>,
}

pub const NOGC_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

impl<VM: VMBinding> Plan for NoGC<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &NOGC_CONSTRAINTS
    }

    fn gc_init(&mut self, heap_size: usize, vm_map: &'static VMMap) {
        self.base.gc_init(heap_size, vm_map);

        // FIXME correctly initialize spaces based on options
        self.nogc_space.init(&vm_map);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base().collection_required(self, space_full, space)
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn prepare(&mut self, _tls: VMWorkerThread) {
        unreachable!()
    }

    fn release(&mut self, _tls: VMWorkerThread) {
        unreachable!()
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        eprintln!("nogc::get_alloc_mapping");
        &*ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, _scheduler: &GCWorkScheduler<VM>) {
        unreachable!("GC triggered in nogc")
    }

    fn get_pages_used(&self) -> usize {
        self.nogc_space.reserved_pages()
    }

    fn handle_user_collection_request(&self, _tls: VMMutatorThread, _force: bool) {
        println!("Warning: User attempted a collection request, but it is not supported in NoGC. The request is ignored.");
    }
}

impl<VM: VMBinding> NoGC<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self {
        #[cfg(not(feature = "nogc_lock_free"))]
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);
        #[cfg(feature = "nogc_lock_free")]
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        let global_specs = SideMetadataContext::new_global_specs(&[]);
        let heap = HeapMeta::new(HEAP_START, HEAP_END);

        let global_specs = SideMetadataContext::new_global_specs(&[]);

        // #[cfg(feature = "nogc_lock_free")]
        // let nogc_space = NoGCImmortalSpace::new(
        //     "nogc_space",
        //     cfg!(not(feature = "nogc_no_zeroing")),
        //     global_specs.clone(),
        // );
        // #[cfg(not(feature = "nogc_lock_free"))]
        // let nogc_space = NoGCImmortalSpace::new(
        //     "nogc_space",
        //     true,
        //     VMRequest::discontiguous(),
        //     global_specs.clone(),
        //     vm_map,
        //     mmapper,
        //     &mut heap,
        //     &NOGC_CONSTRAINTS,
        // );
        let nogc_space = MarkSweepSpace::new(
            "MSspace",
            true,
            VMRequest::discontiguous(),
            global_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
        );

        let res = NoGC {
            nogc_space,
            base: BasePlan::new(
                vm_map,
                mmapper,
                options,
                heap,
                &NOGC_CONSTRAINTS,
                global_specs,
            ),
        };

        // Use SideMetadataSanity to check if each spec is valid. This is also needed for check
        // side metadata in extreme_assertions.
        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        res.base()
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.nogc_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        eprintln!("NoGC plan");
        res
    }
}