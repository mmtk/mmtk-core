use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::vm::Collection;
use ::util::heap::MonotonePageResource;
use ::util::OpaquePointer;

use plan::semispace::SemiSpace;
use vm::VMBinding;

#[repr(C)]
pub struct SSMutator<VM: VMBinding> {
    // CopyLocal
    ss: BumpAllocator<VM, MonotonePageResource<VM, CopySpace<VM>>>,
    vs: BumpAllocator<VM, MonotonePageResource<VM, ImmortalSpace<VM>>>,
    los: LargeObjectAllocator<VM>,

    plan: &'static SemiSpace<VM>
}

impl<VM: VMBinding> MutatorContext for SSMutator<VM> {
    fn collection_phase(&mut self, _tls: OpaquePointer, phase: &Phase, _primary: bool) {
        match phase {
            &Phase::PrepareStacks => {
                if !self.plan.common.stacks_prepared() {
                    VM::VMCollection::prepare_mutator(self.ss.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {}
            &Phase::Release => {
                // rebind the allocation bump pointer to the appropriate semispace
                self.ss.rebind(Some(self.plan.tospace()));
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, _type_refer: ObjectReference, _bytes: usize, allocator: AllocationType) {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => {}
            AllocationType::Los => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.los.get_space().unwrap().initialize_header(refer, true);
            }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.vs.get_space().unwrap().initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        debug_assert!(self.ss.tls == self.vs.tls);
        debug_assert!(self.ss.tls == self.los.tls);
        self.ss.tls
    }
}

impl<VM: VMBinding> SSMutator<VM> {
    pub fn new(tls: OpaquePointer, plan: &'static SemiSpace<VM>) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(plan.tospace()), plan),
            vs: BumpAllocator::new(tls, Some(plan.get_versatile_space()), plan),
            los: LargeObjectAllocator::new(tls, Some(plan.get_los()), plan),
            plan
        }
    }
}