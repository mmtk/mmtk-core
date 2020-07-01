use crate::plan::plan::Plan;
use crate::plan::plan::CommonPlan;
use crate::plan::selected_plan::SelectedPlan;
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::vm::Collection;

use enum_map::EnumMap;

pub struct MutatorConfig<VM: VMBinding, P: Plan<VM> + 'static> {
    // All the allocators for this mutator
    pub allocators: Vec<Box<dyn Allocator<VM>>>,
    // Mapping between allocation semantics and allocator index
    pub allocator_mapping: EnumMap<AllocationType, usize>,
    // Plan-specific code for mutator collection phase
    pub collection_phase_func: &'static dyn Fn(&mut Mutator<VM, P>, OpaquePointer, &Phase, bool),
}

impl<VM: VMBinding, P: Plan<VM> + 'static> MutatorConfig<VM, P> {
    pub fn get_allocator(&self, allocator: AllocationType) -> &dyn Allocator<VM> {
        let allocator_index = self.allocator_mapping[allocator];
        self.allocators[allocator_index].as_ref()
    }

    pub fn get_allocator_mut(&mut self, allocator: AllocationType) -> &mut dyn Allocator<VM> {
        let allocator_index = self.allocator_mapping[allocator];
        self.allocators[allocator_index].as_mut()
    }
}

pub struct Mutator<VM: VMBinding, P: Plan<VM> + 'static> {
    pub mutator_tls: OpaquePointer,
    pub config: MutatorConfig<VM, P>,
    // pub common: CommonMutatorContext<VM>,
    pub plan: &'static P,
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
        self.config.get_allocator_mut(allocator).alloc(size, align, offset)
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        self.config.get_allocator(allocator).get_space().unwrap().initialize_header(refer, true)
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
