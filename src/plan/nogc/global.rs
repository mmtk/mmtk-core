use crate::mmtk::MMTK;
use crate::plan::global::{BasePlan, NoCopy};
use crate::plan::nogc::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::GCWorkScheduler;
use crate::scheduler::GCWorkerLocal;
use crate::scheduler::GCWorkerLocalPtr;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::vm_layout_constants::{HEAP_END, HEAP_START};
use crate::util::heap::HeapMeta;
#[allow(unused_imports)]
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{LOCAL_SIDE_METADATA_BASE_ADDRESS, SideMetadataContext, SideMetadataSanity, SideMetadataSpec, metadata_address_range_size};
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
    pub nogc_space: NoGCImmortalSpace<VM>,
}

pub const NOGC_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

impl<VM: VMBinding> Plan for NoGC<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &NOGC_CONSTRAINTS
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
        self.base.gc_init(heap_size, vm_map, scheduler);

        // FIXME correctly initialize spaces based on options
        self.nogc_space.init(&vm_map);
    }

    fn collection_required(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        self.base.collection_required(self, space_full, space)
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.base
    }

    fn prepare(&mut self, _tls: VMWorkerThread) {
        unreachable!()
    }

    fn release(&mut self, tls: VMWorkerThread) {
        self.ms_space.eager_sweep(tls);
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn schedule_collection(&'static self, _scheduler: &GCWorkScheduler<VM>) {
        unreachable!("GC triggered in nogc")
    }

    fn get_pages_used(&self) -> usize {
        self.im_space.reserved_pages() + self.ms_space.reserved_pages()
    }

    fn handle_user_collection_request(&self, _tls: VMMutatorThread, _force: bool) {
        println!("Warning: User attempted a collection request, but it is not supported in NoGC. The request is ignored.");
    }

    fn poll(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        false
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
        let heap = HeapMeta::new(HEAP_START, HEAP_END);
        let side_metadata_next = SideMetadataSpec {
            is_global: false,
            offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_size = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_local_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        let side_metadata_thread_free = SideMetadataSpec {
            is_global: false,
            offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free),
            log_num_of_bits: 6,
            log_min_obj_size: 16,
        };
        // let side_metadata_tls = SideMetadataSpec {
        //     is_global: false,
        //     offset: metadata_address_range_size(&side_metadata_next) + metadata_address_range_size(&side_metadata_free) + metadata_address_range_size(&side_metadata_size) + metadata_address_range_size(&side_metadata_local_free) + metadata_address_range_size(&side_metadata_thread_free),
        //     log_num_of_bits: 6,
        //     log_min_obj_size: 16,
        // };
        let local_specs = {
            vec![
                side_metadata_next,
                side_metadata_free,
                side_metadata_size,
                side_metadata_local_free,
                side_metadata_thread_free,
            ]
        };

        #[cfg(feature = "nogc_lock_free")]
        let nogc_space = NoGCImmortalSpace::new(
            "nogc_space",
            cfg!(not(feature = "nogc_no_zeroing")),
            global_specs.clone(),
        );
        #[cfg(not(feature = "nogc_lock_free"))]
        let nogc_space = NoGCImmortalSpace::new(
            "nogc_space",
            true,
            VMRequest::discontiguous(),
            // local_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
            &NOGC_CONSTRAINTS,
        );
        let global_specs = SideMetadataContext::new_global_specs(&[]);

        let im_space = ImmortalSpace::new(
            "IMspace",
            true,
            VMRequest::discontiguous(),
            global_specs.clone(),
            vm_map,
            mmapper,
            &mut heap,
            &NOGC_CONSTRAINTS,
        );

        let res = NoGC {
            im_space,
            ms_space,
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
        res.base
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.ms_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res.im_space
            .verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        res
    }
}