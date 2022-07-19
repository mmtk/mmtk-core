pub(super) use super::super::ALLOCATOR_MAPPING;
use super::GenCopy;
use crate::plan::barriers::*;
use crate::plan::generational::create_gen_space_mapping;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::mutator_context::Mutator;
use crate::plan::mutator_context::MutatorConfig;
use crate::plan::AllocationSemantics;
use crate::util::alloc::allocators::Allocators;
use crate::util::alloc::BumpAllocator;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;

pub fn gencopy_mutator_prepare<VM: VMBinding>(_mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // Do nothing
}

pub fn gencopy_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, _tls: VMWorkerThread) {
    // reset nursery allocator
    let bump_allocator = unsafe {
        mutator
            .allocators
            .get_allocator_mut(mutator.config.allocator_mapping[AllocationSemantics::Default])
    }
    .downcast_mut::<BumpAllocator<VM>>()
    .unwrap();
    bump_allocator.reset();
}

pub fn create_gencopy_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let gencopy = mmtk.plan.downcast_ref::<GenCopy<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: Box::new(create_gen_space_mapping(&*mmtk.plan, &gencopy.gen.nursery)),
        prepare_func: &gencopy_mutator_prepare,
        release_func: &gencopy_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, &*mmtk.plan, &config.space_mapping),
        barrier: Box::new(ObjectRememberingBarrier::<GenNurseryProcessEdges<VM>>::new(
            mmtk,
            *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
        )),
        mutator_tls,
        config,
        plan: gencopy,
    }
}
