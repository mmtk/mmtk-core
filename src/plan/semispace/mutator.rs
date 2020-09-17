use crate::plan::mutator_context::{CommonMutatorContext, MutatorContext};
use crate::plan::Allocator as AllocationType;
use crate::plan::Phase;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::plan::semispace::SemiSpace;
use crate::vm::VMBinding;

#[repr(C)]
pub struct SSMutator<VM: VMBinding> {
    ss: BumpAllocator<VM>,
    plan: &'static SemiSpace<VM>,
    common: CommonMutatorContext<VM>,
}

impl<VM: VMBinding> MutatorContext<VM> for SSMutator<VM> {
    fn common(&self) -> &CommonMutatorContext<VM> {
        &self.common
    }

    fn prepare(&mut self, _tls: OpaquePointer) {
        // Do nothing
    }

    fn release(&mut self, _tls: OpaquePointer) {
        self.ss.rebind(Some(self.plan.tospace()));
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
            self.ss.get_space().unwrap().common().descriptor
                == self.plan.tospace().common().descriptor,
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
        debug_assert!(
            self.ss.get_space().unwrap().common().descriptor
                == self.plan.tospace().common().descriptor
        );
        match allocator {
            AllocationType::Default => {}
            _ => self.common.post_alloc(object, _type, _bytes, allocator),
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.ss.tls
    }
}

impl<VM: VMBinding> SSMutator<VM> {
    pub fn new(tls: OpaquePointer, plan: &'static SemiSpace<VM>) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(plan.tospace()), plan),
            plan,
            common: CommonMutatorContext::<VM>::new(tls, plan, &plan.common),
        }
    }
}
