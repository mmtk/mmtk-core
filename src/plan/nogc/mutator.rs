use crate::plan::mutator_context::{CommonMutatorContext, MutatorContext};
use crate::plan::nogc::NoGC;
use crate::plan::Allocator as AllocationType;
use crate::util::alloc::Allocator;
use crate::util::alloc::BumpAllocator;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

#[repr(C)]
pub struct NoGCMutator<VM: VMBinding> {
    // ImmortalLocal
    nogc: BumpAllocator<VM>,
}

impl<VM: VMBinding> MutatorContext<VM> for NoGCMutator<VM> {
    fn common(&self) -> &CommonMutatorContext<VM> {
        unreachable!()
    }

    fn prepare(&mut self, _tls: OpaquePointer) {
        unreachable!()
    }
    fn release(&mut self, _tls: OpaquePointer) {
        unreachable!()
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
        self.nogc.alloc(size, align, offset)
    }

    // We may match other patterns in the future, so temporarily disable this check
    #[allow(clippy::single_match)]
    #[allow(clippy::match_single_binding)]
    fn post_alloc(
        &mut self,
        _refer: ObjectReference,
        _type_refer: ObjectReference,
        _bytes: usize,
        allocator: AllocationType,
    ) {
        match allocator {
            // FIXME: other allocation types
            _ => {}
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.nogc.tls
    }
}

impl<VM: VMBinding> NoGCMutator<VM> {
    pub fn new(tls: OpaquePointer, plan: &'static NoGC<VM>) -> Self {
        NoGCMutator {
            nogc: BumpAllocator::new(tls, Some(&plan.nogc_space), plan),
        }
    }
}
