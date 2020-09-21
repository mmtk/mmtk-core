use crate::plan::mutator_context::{CommonMutatorContext, MutatorContext};
use crate::plan::Allocator as AllocationType;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use super::GenCopy;
use crate::vm::VMBinding;

#[repr(C)]
pub struct GenCopyMutator<VM: VMBinding> {
    ss: BumpAllocator<VM>,
    plan: &'static GenCopy<VM>,
    common: CommonMutatorContext<VM>,
}

impl <VM: VMBinding> MutatorContext<VM> for GenCopyMutator<VM> {
    fn common(&self) -> &CommonMutatorContext<VM> {
        &self.common
    }

    fn prepare(&mut self, _tls: OpaquePointer) {
        // Do nothing
    }

    fn release(&mut self, _tls: OpaquePointer) {
        self.ss.rebind(Some(&self.plan.nursery));
    }

    fn alloc(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        allocator: AllocationType,
    ) -> Address {
        trace!(
            "MutatorContext.alloc({}, {}, {}, {:?})",
            size,
            align,
            offset,
            allocator
        );
        debug_assert!(
            self.ss.get_space().unwrap() as *const _ == &self.plan.nursery as *const _,
            "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
            self as *const _,
            self.ss.get_space().unwrap() as *const _,
            self.plan.tospace() as *const _
        );
        match allocator {
            AllocationType::Default => self.ss.alloc(size, align, offset),
            _ => self.common.alloc(size, align, offset, allocator),
        }
    }

    fn post_alloc(
        &mut self,
        object: ObjectReference,
        _type: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        // debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => {}
            _ => self.common.post_alloc(object, _type, _bytes, allocator),
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.ss.tls
    }
}

impl <VM: VMBinding> GenCopyMutator<VM> {
    pub fn new(tls: OpaquePointer, plan: &'static GenCopy<VM>) -> Self {
        Self {
            ss: BumpAllocator::new(tls, Some(&plan.nursery), plan),
            plan,
            common: CommonMutatorContext::<VM>::new(tls, plan, &plan.common),
        }
    }
}
