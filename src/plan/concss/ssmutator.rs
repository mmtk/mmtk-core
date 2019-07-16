use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::policy::largeobjectspace::LargeObjectSpace;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::plan::plan;
use ::vm::{Collection, VMCollection};
use ::util::heap::{PageResource, MonotonePageResource};
use super::PLAN;
use ::util::queue::LocalQueue;
use libc::c_void;
use policy::space::Space;
use std::sync::atomic::{Ordering, AtomicUsize};

#[repr(C)]
pub struct SSMutator {
    ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    los: LargeObjectAllocator,
}

impl MutatorContext for SSMutator {
    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, _primary: bool) {
        // println!("Mutator {:?}", phase);
        match phase {
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.ss.reset();
                self.flush_remembered_sets();
            }
            &Phase::Release => {
                self.ss.reset();
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, nursery_space: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        unreachable!()
        // debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.nursery_space() as *const _,
        //               "bumpallocator {:?} holds wrong space, ss.space: {:?}, nursery_space: {:?}",
        //               self as *const _,
        //               self.ss.get_space().unwrap() as *const _,
        //               PLAN.nursery_space() as *const _);
        // match allocator {
        //     AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
        //     AllocationType::Los => { self.los.alloc(size, align, offset) }
        //     _ => { self.vs.alloc_slow(size, align, offset) }
        // }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => {
            }
            AllocationType::Los => {
                PLAN.los.initialize_header(refer, true);
            }
            _ => {
                PLAN.versatile_space.initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> *mut c_void {
        debug_assert!(self.ss.tls == self.vs.tls);
        self.ss.tls
    }

    fn flush_remembered_sets(&mut self) {
        // self.remset.flush();
    }

    fn object_reference_write_slow(&mut self, _src: ObjectReference, _slot: Address, _value: ObjectReference) {
        
    }
    fn object_reference_read_slow(&mut self, _src: ObjectReference, slot: Address) -> ObjectReference {
        unsafe { slot.load() }
    }
    fn object_reference_try_compare_and_swap_slow(&mut self, _src: ObjectReference, slot: Address, old: ObjectReference, new: ObjectReference) -> bool {
        let slot = unsafe { ::std::mem::transmute::<Address, &AtomicUsize>(slot) };
        return slot.compare_and_swap(old.to_address().as_usize(), new.to_address().as_usize(), Ordering::Relaxed) == old.to_address().as_usize()
    }
    fn java_lang_reference_read_slow(&mut self, obj: ObjectReference) -> ObjectReference {
        obj
    }
}

impl SSMutator {
    pub fn new(tls: *mut c_void, space: &'static CopySpace, versatile_space: &'static ImmortalSpace, los: &'static LargeObjectSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(space)),
            vs: BumpAllocator::new(tls, Some(versatile_space)),
            los: LargeObjectAllocator::new(tls, Some(los)),
        }
    }
}