use crate::plan::global::Plan;
use crate::plan::global::CommonPlan;
use crate::plan::selected_plan::SelectedPlan;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};
use crate::util::alloc::allocators::{Allocators, AllocatorSelector};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::vm::Collection;
use crate::policy::space::Space;

use enum_map::EnumMap;

pub struct MutatorConfig<VM: VMBinding, P: Plan<VM> + 'static> {
    // Mapping between allocation semantics and allocator selector
    pub allocator_mapping: EnumMap<AllocationType, AllocatorSelector>,    
    // Mapping between allocator selector and spaces. Each pair represents a mapping.
    pub space_mapping: Vec<(AllocatorSelector, &'static dyn Space<VM>)>,
    // Plan-specific code for mutator collection phase
    pub collection_phase_func: &'static dyn Fn(&mut Mutator<VM, P>, OpaquePointer, &Phase, bool),
}

pub struct Mutator<VM: VMBinding, P: Plan<VM> + 'static> {
    pub allocators: Allocators<VM>,
    pub mutator_tls: OpaquePointer,
    // pub common: CommonMutatorContext<VM>,
    pub plan: &'static P,
    pub config: MutatorConfig<VM, P>,
}

impl<VM: VMBinding, P: Plan<VM>> MutatorContext<VM> for Mutator<VM, P> {
    // fn common(&self) -> &CommonMutatorContext<VM> {
    //     unreachable!()
    // }

    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool) {
        match phase {
            Phase::PrepareStacks => {
                if !self.plan.common().stacks_prepared() {
                    // Use the mutator's tls rather than the collector's tls
                    VM::VMCollection::prepare_mutator(self.get_tls(), self);
                }
                self.flush_remembered_sets();
            }
            // Ignore for other phases
            _ => {},
        }
        (*self.config.collection_phase_func)(self, tls, phase, primary)
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        unsafe { self.allocators.get_allocator_mut(self.config.allocator_mapping[allocator]) }.alloc(size, align, offset)
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        unsafe { self.allocators.get_allocator_mut(self.config.allocator_mapping[allocator]) }.get_space().unwrap().initialize_header(refer, true)
    }

    fn get_tls(&self) -> OpaquePointer {
        self.mutator_tls
    }
}

pub trait MutatorContext<VM: VMBinding> {
    // fn common(&self) -> &CommonMutatorContext<VM>;
    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool);
    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address;
    fn post_alloc(
        &mut self,
        refer: ObjectReference,
        type_refer: ObjectReference,
        bytes: usize,
        allocator: AllocationType,
    );
    fn flush_remembered_sets(&mut self) {}
    fn get_tls(&self) -> OpaquePointer;
}

// pub struct CommonMutatorContext<VM: VMBinding> {
//     immortal: BumpAllocator<VM>,
//     los: LargeObjectAllocator<VM>,
// }

// impl<VM: VMBinding> CommonMutatorContext<VM> {
//     pub fn new(
//         tls: OpaquePointer,
//         plan: &'static SelectedPlan<VM>,
//         common_plan: &'static CommonPlan<VM>,
//     ) -> Self {
//         CommonMutatorContext {
//             immortal: BumpAllocator::new(tls, Some(common_plan.get_immortal()), plan),
//             los: LargeObjectAllocator::new(tls, Some(common_plan.get_los()), plan),
//         }
//     }

//     pub fn alloc(
//         &mut self,
//         size: usize,
//         align: usize,
//         offset: isize,
//         allocator: AllocationType,
//     ) -> Address {
//         match allocator {
//             AllocationType::Los => self.los.alloc(size, align, offset),
//             AllocationType::Immortal => self.immortal.alloc(size, align, offset),
//             _ => panic!("Unexpected allocator for alloc(): {:?}", allocator),
//         }
//     }

//     pub fn post_alloc(
//         &mut self,
//         object: ObjectReference,
//         _type: ObjectReference,
//         _bytes: usize,
//         allocator: AllocationType,
//     ) {
//         match allocator {
//             AllocationType::Los => {
//                 self.los
//                     .get_space()
//                     .unwrap()
//                     .initialize_header(object, true);
//             }
//             AllocationType::Immortal => self
//                 .immortal
//                 .get_space()
//                 .unwrap()
//                 .initialize_header(object, true),
//             _ => panic!("Unexpected allocator for post_alloc(): {:?}", allocator),
//         }
//     }
// }
