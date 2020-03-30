use crate::policy::immortalspace::ImmortalSpace;
use crate::util::alloc::{BumpAllocator, LargeObjectAllocator};
use crate::plan::mutator_context::MutatorContext;
use crate::plan::Phase;
use crate::util::{Address, ObjectReference};
use crate::util::alloc::Allocator;
use crate::plan::Allocator as AllocationType;
use crate::util::heap::MonotonePageResource;
use crate::util::OpaquePointer;
use crate::plan::nogc::NoGC;
use crate::vm::VMBinding;

#[repr(C)]
pub struct NoGCMutator<VM: VMBinding> {
    // ImmortalLocal
    nogc: BumpAllocator<VM, MonotonePageResource<VM, ImmortalSpace<VM>>>,
    los: LargeObjectAllocator<VM>,
}

impl<VM: VMBinding> MutatorContext for NoGCMutator<VM> {
    fn collection_phase(&mut self, _tls: OpaquePointer, _phase: &Phase, _primary: bool) {
        unreachable!()
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        match allocator {
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.nogc.alloc(size, align, offset)
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        match allocator {
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.nogc.alloc(size, align, offset)
        }
    }

    // We may match other patterns in the future, so temporarily disable this check
    #[allow(clippy::single_match)]
    fn post_alloc(&mut self, refer: ObjectReference, _type_refer: ObjectReference, _bytes: usize, allocator: AllocationType) {
        match allocator {
            AllocationType::Los => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.los.get_space().unwrap().initialize_header(refer, true);
            }
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
            nogc: BumpAllocator::new(tls, Some(plan.get_immortal_space()), plan),
            los: LargeObjectAllocator::new(tls, Some(plan.get_los()), plan),
        }
    }
}