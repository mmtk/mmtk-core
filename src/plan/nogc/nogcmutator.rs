use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::util::heap::MonotonePageResource;
use ::util::OpaquePointer;
use plan::nogc::NoGC;
use vm::VMBinding;

#[repr(C)]
pub struct NoGCMutator<VM: VMBinding> {
    // ImmortalLocal
    nogc: BumpAllocator<VM, MonotonePageResource<VM, ImmortalSpace<VM>>>,
    los: LargeObjectAllocator<VM>,
}

impl<VM: VMBinding> MutatorContext for NoGCMutator<VM> {
    fn collection_phase(&mut self, _tls: OpaquePointer, _phase: &Phase, _primary: bool) {
        unimplemented!();
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