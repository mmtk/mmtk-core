use ::policy::immortalspace::ImmortalSpace;
use ::policy::largeobjectspace::LargeObjectSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::util::heap::MonotonePageResource;
use ::mmtk::SINGLETON;
use ::util::OpaquePointer;
use libc::c_void;
use plan::nogc::NoGC;

#[repr(C)]
pub struct NoGCMutator {
    // ImmortalLocal
    nogc: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    los: LargeObjectAllocator,
}

impl MutatorContext for NoGCMutator {
    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool) {
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

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
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

impl NoGCMutator {
    pub fn new(tls: OpaquePointer, plan: &'static NoGC) -> Self {
        NoGCMutator {
            nogc: BumpAllocator::new(tls, Some(plan.get_immortal_space()), plan),
            los: LargeObjectAllocator::new(tls, Some(plan.get_los()), plan),
        }
    }
}