use crate::plan::barriers::ObjectBarrier;
use crate::plan::generational::barrier::GenObjectBarrierSemantics;
use crate::plan::immix;
use crate::plan::mutator_context::{create_space_mapping, unreachable_prepare_func, MutatorConfig};
use crate::plan::sticky::immix::global::StickyImmix;
use crate::util::alloc::allocators::Allocators;
use crate::util::alloc::AllocatorSelector;
use crate::util::opaque_pointer::VMWorkerThread;
use crate::util::VMMutatorThread;
use crate::vm::VMBinding;
use crate::{Mutator, MMTK};

pub fn stickyimmix_mutator_release<VM: VMBinding>(mutator: &mut Mutator<VM>, tls: VMWorkerThread) {
    immix::mutator::immix_mutator_release(mutator, tls)
}

pub use immix::mutator::ALLOCATOR_MAPPING;

pub fn create_stickyimmix_mutator<VM: VMBinding>(
    mutator_tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Mutator<VM> {
    let stickyimmix = mmtk.get_plan().downcast_ref::<StickyImmix<VM>>().unwrap();
    let config = MutatorConfig {
        allocator_mapping: &ALLOCATOR_MAPPING,
        space_mapping: Box::new({
            let mut vec =
                create_space_mapping(immix::mutator::RESERVED_ALLOCATORS, true, mmtk.get_plan());
            vec.push((AllocatorSelector::Immix(0), stickyimmix.get_immix_space()));
            vec
        }),
        prepare_func: &unreachable_prepare_func,
        release_func: &stickyimmix_mutator_release,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, mmtk, &config.space_mapping),
        barrier: Box::new(ObjectBarrier::new(GenObjectBarrierSemantics::new(
            mmtk,
            stickyimmix,
        ))),
        mutator_tls,
        config,
        plan: mmtk.get_plan(),
    }
}
