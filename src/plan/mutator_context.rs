use crate::plan::plan::CommonPlan;
use crate::plan::plan::Plan;
use crate::plan::selected_plan::{SelectedMutator, SelectedPlan};
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::policy::immortalspace::ImmortalSpace;
use crate::util::alloc::{Allocator, BumpAllocator, LargeObjectAllocator};
use crate::util::heap::MonotonePageResource;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::vm::VMBinding;

pub trait MutatorContext<VM: VMBinding> {
    fn common(&self) -> &CommonMutatorContext<VM>;
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

pub struct CommonMutatorContext<VM: VMBinding> {
    immortal: BumpAllocator<VM, MonotonePageResource<VM, ImmortalSpace<VM>>>,
    los: LargeObjectAllocator<VM>,
    common_plan: &'static CommonPlan<VM>,
}

impl<VM: VMBinding> CommonMutatorContext<VM> {
    pub fn new(
        tls: OpaquePointer,
        plan: &'static SelectedPlan<VM>,
        common_plan: &'static CommonPlan<VM>,
    ) -> Self {
        CommonMutatorContext {
            immortal: BumpAllocator::new(tls, Some(common_plan.get_immortal()), plan),
            los: LargeObjectAllocator::new(tls, Some(common_plan.get_los()), plan),
            common_plan: common_plan,
        }
    }

    pub fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address {
        match allocator {
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.immortal.alloc(size, align, offset),
        }
    }

    pub fn post_alloc(
        &mut self,
        object: ObjectReference,
        _type: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        match allocator {
            AllocationType::Los => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.los
                    .get_space()
                    .unwrap()
                    .initialize_header(object, true);
            }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.immortal.get_space().unwrap().initialize_header(object);
            }
        }
    }
}
