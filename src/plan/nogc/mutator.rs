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

pub fn nogc_collection_phase<VM: VMBinding>(mutator: &mut Mutator<VM, NoGC<VM>>, tls: OpaquePointer, phase: &Phase, primary: bool) {}

pub fn create_nogc_mutator<VM: VMBinding>(mutator_tls: OpaquePointer, plan: &'static NoGC<VM>) -> Mutator<VM, NoGC<VM>> {
    let config = MutatorConfig {
        // allocators: vec![
        //     // 0 - nogc
        //     Box::new(),
        // ],
        allocator_mapping: enum_map!{
            AllocationType::Default | AllocationType::Immortal | AllocationType::Code | AllocationType::ReadOnly | AllocationType::Los => AllocatorSelector::BumpPointer(0),
        },
        collection_phase_func: &nogc_collection_phase,
    };

    let mut allocators = Allocators::<VM>::uninit();
    allocators.bump_pointer[0].write(BumpAllocator::new(mutator_tls, Some(plan.get_immortal_space()), plan));

    Mutator {
        allocators,
        mutator_tls,
        config,
        plan
    }
}

// #[repr(C)]
// pub struct NoGCMutator<VM: VMBinding> {
//     // ImmortalLocal
//     nogc: BumpAllocator<VM>,
// }

// impl<VM: VMBinding> MutatorContext<VM> for NoGCMutator<VM> {
//     fn common(&self) -> &CommonMutatorContext<VM> {
//         unreachable!()
//     }
    // We may match other patterns in the future, so temporarily disable this check
    // #[allow(clippy::single_match)]
    // #[allow(clippy::match_single_binding)]
    // fn post_alloc(
    //     &mut self,
    //     _refer: ObjectReference,
    //     _type_refer: ObjectReference,
    //     _bytes: usize,
    //     allocator: AllocationType,
    // ) {
    //     match allocator {
    //         // FIXME: other allocation types
    //         _ => {}
    //     }
    // }

//     fn collection_phase(&mut self, _tls: OpaquePointer, _phase: &Phase, _primary: bool) {
//         unreachable!()
//     }

//     fn alloc(
//         &mut self,
//         size: usize,
//         align: usize,
//         offset: isize,
//         allocator: AllocationType,
//     ) -> Address {
//         trace!(
//             "MutatorContext.alloc({}, {}, {}, {:?})",
//             size,
//             align,
//             offset,
//             allocator
//         );
//         self.nogc.alloc(size, align, offset)
//     }

//     // We may match other patterns in the future, so temporarily disable this check
//     #[allow(clippy::single_match)]
//     fn post_alloc(
//         &mut self,
//         _refer: ObjectReference,
//         _type_refer: ObjectReference,
//         _bytes: usize,
//         allocator: AllocationType,
//     ) {
//         match allocator {
//             // FIXME: other allocation types
//             _ => {}
//         }
//     }

//     fn get_tls(&self) -> OpaquePointer {
//         self.nogc.tls
//     }
// }

// impl<VM: VMBinding> NoGCMutator<VM> {
//     pub fn new(tls: OpaquePointer, plan: &'static NoGC<VM>) -> Self {
//         NoGCMutator {
//             nogc: BumpAllocator::new(tls, Some(plan.get_immortal_space()), plan),
//         }
//     }
// }
