use crate::plan::barriers::SATBBarrier;
use crate::plan::concurrent::barrier::SATBBarrierSemantics;
use crate::plan::concurrent::immix::ConcurrentImmix;
use crate::plan::concurrent::Pause;
use crate::plan::mutator_context::create_allocator_mapping;
use crate::plan::mutator_context::create_space_mapping;

use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorBuilder;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::mutator_context::ReservedAllocators;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::alloc::ImmixAllocator;
use crate::util::opaque_pointer::{VMMutatorThread, VMWorkerThread};
use crate::vm::VMBinding;
use crate::MMTK;
use enum_map::EnumMap;

pub fn concurrent_immix_mutator_release<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // Release is not scheduled for initial mark pause
    let current_pause = mutator.plan.concurrent().unwrap().current_pause().unwrap();
    debug_assert_ne!(current_pause, Pause::InitialMark);

    let immix_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<ImmixAllocator<VM>>()
    .unwrap();
    immix_allocator.reset();

    // Deactivate SATB
    if current_pause == Pause::Full || current_pause == Pause::FinalMark {
        debug!("Deactivate SATB barrier active for {:?}", mutator as *mut _);
        mutator.barrier.set_active(false);
    }
}

pub fn concurent_immix_mutator_prepare<VM: VMBinding>(
    mutator: &mut Mutator<VM>,
    _tls: VMWorkerThread,
) {
    // Prepare is not scheduled for final mark pause
    let current_pause = mutator.plan.concurrent().unwrap().current_pause().unwrap();
    debug_assert_ne!(current_pause, Pause::FinalMark);

    let immix_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<ImmixAllocator<VM>>()
    .unwrap();
    immix_allocator.reset();

    // Activate SATB
    if current_pause == Pause::InitialMark {
        debug!("Activate SATB barrier active for {:?}", mutator as *mut _);
        mutator.barrier.set_active(true);
    }
}

pub(in crate::plan) const RESERVED_ALLOCATORS: ReservedAllocators = ReservedAllocators {
    n_immix: 1,
    ..ReservedAllocators::DEFAULT
};

lazy_static! {
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationSemantics, AllocatorSelector> = {
        let mut map = create_allocator_mapping(RESERVED_ALLOCATORS, true);
        map[AllocationSemantics::Default] = AllocatorSelector::Immix(0);
        map
    };
}

pub fn create_concurrent_immix_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let immix = mmtk
        .get_plan()
        .downcast_ref::<ConcurrentImmix<VM>>()
        .unwrap();
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec = create_space_mapping(RESERVED_ALLOCATORS, true, immix);
            vec.push((AllocatorSelector::Immix(0), &immix.immix_space));
            vec
        }),

        prepare_func: &concurent_immix_mutator_prepare,
        release_func: &concurrent_immix_mutator_release,
    };

    let builder = MutatorBuilder::new(mutator_tls, mmtk, config);
    let mut mutator = builder
        .barrier(Box::new(SATBBarrier::new(SATBBarrierSemantics::<
            VM,
            ConcurrentImmix<VM>,
            { crate::policy::immix::TRACE_KIND_FAST },
        >::new(mmtk, mutator_tls))))
        .build();

    // Set barrier active, based on whether concurrent marking is in progress
    mutator
        .barrier
        .set_active(immix.is_concurrent_marking_active());

    mutator
}
