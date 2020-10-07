use crate::plan::mutator_context::MutatorContext;
use crate::plan::mutator_context::Mutator;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::alloc::LargeObjectAllocator;
use crate::util::alloc::allocators::{Allocators, AllocatorSelector};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;

use crate::plan::semispace::SemiSpace;
use crate::vm::VMBinding;
use crate::plan::mutator_context::MutatorConfig;
use downcast_rs::Downcast;
use enum_map::enum_map;
use enum_map::EnumMap;

pub fn ss_collection_phase<VM: VMBinding>(mutator: &mut Mutator<VM, SemiSpace<VM>>, tls: OpaquePointer, phase: &Phase, primary: bool) {
    match phase {
        Phase::Release => {
            // rebind the allocation bump pointer to the appropriate semispace
            let bump_allocator = unsafe { mutator.allocators.get_allocator_mut(mutator.config.allocator_mapping[AllocationType::Default]) }.downcast_mut::<BumpAllocator<VM>>().unwrap();
            bump_allocator.rebind(Some(mutator.plan.tospace()));
        }
        _ => {}
    }
}

lazy_static!{
    pub static ref ALLOCATOR_MAPPING: EnumMap<AllocationType, AllocatorSelector> = enum_map!{
        AllocationType::Default => AllocatorSelector::BumpPointer(0),
        AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly => AllocatorSelector::BumpPointer(1),
        AllocationType::Los => AllocatorSelector::LargeObject(0),
    };
}

pub fn create_ss_mutator<VM: VMBinding>(mutator_tls: OpaquePointer, plan: &'static SemiSpace<VM>) -> Mutator<VM, SemiSpace<VM>> {
    let config = MutatorConfig {
        allocator_mapping: &*ALLOCATOR_MAPPING,
        space_mapping: vec![
            (AllocatorSelector::BumpPointer(0), plan.fromspace()),
            (AllocatorSelector::BumpPointer(1), plan.common.get_immortal()),
            (AllocatorSelector::LargeObject(0), plan.common.get_los()),
        ],
        collection_phase_func: &ss_collection_phase,
    };

    Mutator {
        allocators: Allocators::<VM>::new(mutator_tls, plan, &config.space_mapping),
        mutator_tls,
        config,
        plan
    }
}

// #[repr(C)]
// pub struct SSMutator<VM: VMBinding> {
//     ss: BumpAllocator<VM>,
//     plan: &'static SemiSpace<VM>,
//     common: CommonMutatorContext<VM>,
// }

// impl<VM: VMBinding> MutatorContext<VM> for SSMutator<VM> {
//     fn common(&self) -> &CommonMutatorContext<VM> {
//         &self.common
//     }

//     fn collection_phase(&mut self, _tls: OpaquePointer, phase: &Phase, _primary: bool) {
//         match phase {
//             Phase::PrepareStacks => {
//                 if !self.plan.common.stacks_prepared() {
//                     // Use the mutator's tls rather than the collector's tls
//                     VM::VMCollection::prepare_mutator(self.get_tls(), self);
//                 }
//                 self.flush_remembered_sets();
//             }
//             Phase::Prepare => {}
//             Phase::Release => {
//                 // rebind the allocation bump pointer to the appropriate semispace
//                 self.ss.rebind(Some(self.plan.tospace()));
//             }
//             _ => panic!("Per-mutator phase not handled!"),
//         }
//     }
    // fn alloc(
    //     &mut self,
    //     size: usize,
    //     align: usize,
    //     offset: isize,
    //     allocator: AllocationType,
    // ) -> Address {
    //     trace!(
    //         "MutatorContext.alloc({}, {}, {}, {:?})",
    //         size,
    //         align,
    //         offset,
    //         allocator
    //     );
    //     debug_assert!(
    //         self.ss.get_space().unwrap().common().descriptor
    //             == self.plan.tospace().common().descriptor,
    //         "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
    //         self as *const _,
    //         self.ss.get_space().unwrap() as *const _,
    //         self.plan.tospace() as *const _
    //     );
    //     match allocator {
    //         AllocationType::Default => self.ss.alloc(size, align, offset),
    //         _ => self.common.alloc(size, align, offset, allocator),
    //     }
    // }

    // fn post_alloc(
    //     &mut self,
    //     object: ObjectReference,
    //     _type: ObjectReference,
    //     _bytes: usize,
    //     allocator: AllocationType,
    // ) {
    //     debug_assert!(
    //         self.ss.get_space().unwrap().common().descriptor
    //             == self.plan.tospace().common().descriptor
    //     );
    //     match allocator {
    //         AllocationType::Default => {}
    //         _ => self.common.post_alloc(object, _type, _bytes, allocator),
    //     }
    // }

//     fn get_tls(&self) -> OpaquePointer {
//         self.ss.tls
//     }
// }

// impl<VM: VMBinding> SSMutator<VM> {
//     pub fn new(tls: OpaquePointer, plan: &'static SemiSpace<VM>) -> Self {
//         SSMutator {
//             ss: BumpAllocator::new(tls, Some(plan.tospace()), plan),
//             plan,
//             common: CommonMutatorContext::<VM>::new(tls, plan, &plan.common),
//         }
//     }
// }
