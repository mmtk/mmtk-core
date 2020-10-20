use crate::plan::mutator_context::MutatorContext;
use crate::plan::mutator_context::Mutator;
use crate::plan::nogc::NoGC;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::alloc::allocators::{Allocators, AllocatorSelector};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

use crate::plan::mutator_context::MutatorConfig;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn nogc_collection_phase<VM: VMBinding>(mutator: &mut Mutator<VM, NoGC<VM>>, tls: OpaquePointer, phase: &Phase, primary: bool) {}

lazy_static!{
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map!{
        AllocationType::Default | AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly | AllocationType::Los => AllocatorSelector::BumpPointer(0),
    };
}

pub fn create_nogc_mutator<VM: VMBinding>(mutator_tls: OpaquePointer, plan: &'static NoGC<VM>) -> Mutator<VM, NoGC<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: box vec![
            (AllocatorSelector::BumpPointer(0), plan.get_immortal_space()),
        ],
        collection_phase_func: &nogc_collection_phase,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        mutator_tls,
        config,
        plan
    }
}
