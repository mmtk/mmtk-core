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
use ::vm::*;
use ::util::heap::{PageResource, MonotonePageResource};
use super::PLAN;
use ::util::queue::LocalQueue;
use libc::c_void;
use policy::space::Space;
use std::sync::atomic::{Ordering, AtomicUsize};
use ::util::forwarding_word as ForwardingWord;
use ::util::forwarding_word::clear_forwarding_bits;

#[repr(C)]
pub struct SSMutator {
    ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    los: LargeObjectAllocator,
    modbuf: Box<LocalQueue<'static, ObjectReference>>,
    trace: Box<LocalQueue<'static, ObjectReference>>,
    barrier_active: usize,
    // _padding: usize,
}

impl MutatorContext for SSMutator {
    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, _primary: bool) {
        if super::VERBOSE {
            println!("Mutator {:?}", phase);
        }
        match phase {
            &Phase::SetBarrierActive => {
                self.ss.rebind(Some(PLAN.tospace()));
                self.flush();
                self.barrier_active = 1;
            }
            &Phase::ClearBarrierActive => {
                self.barrier_active = 0;
            }
            &Phase::FlushMutator => {
                self.ss.rebind(Some(PLAN.tospace()));
                self.flush();
            }
            &Phase::FinalClosure => {
                self.ss.rebind(Some(PLAN.tospace()));
                self.flush();
            }
            &Phase::PrepareStacks => {
                self.ss.rebind(Some(PLAN.tospace()));
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.ss.rebind(Some(PLAN.tospace()));
                self.flush_remembered_sets();
            }
            &Phase::Release => {
                self.ss.rebind(Some(PLAN.tospace()));
                self.flush();
            }
            &Phase::ValidatePrepare => {
                self.ss.reset();
            }
            &Phase::ValidateRelease => {
                self.ss.reset();
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => self.ss.alloc(size, align, offset),
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.vs.alloc(size, align, offset),
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => self.ss.alloc_slow(size, align, offset),
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.vs.alloc_slow(size, align, offset),
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
         debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, to: {:?}, from: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _,
                      PLAN.fromspace() as *const _);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _);
        clear_forwarding_bits(refer);
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
        self.modbuf.flush();
        self.trace.flush();
    }

     fn object_reference_write_slow(&mut self, mut src: ObjectReference, mut slot: Address, mut value: ObjectReference) {
        debug_assert!(self.barrier_active());

        let old = unsafe { slot.load::<ObjectReference>() };
        self.check_and_enqueue_reference(old);

        unsafe { slot.store(value) }
    }

    fn object_reference_try_compare_and_swap_slow(&mut self, mut src: ObjectReference, slot: Address, old: ObjectReference, mut tgt: ObjectReference) -> bool {
        debug_assert!(self.barrier_active());
        
        let mut result = compare_and_swap(slot, old, tgt);
        if !result {
            result = compare_and_swap(slot, self.forward(old), tgt);
        }
        self.check_and_enqueue_reference(old);
        return result;
    }

    fn java_lang_reference_read_slow(&mut self, mut obj: ObjectReference) -> ObjectReference {
        debug_assert!(self.barrier_active());
        self.check_and_enqueue_reference(obj);
        self.forward(obj)
    }

    fn object_reference_read_slow(&mut self, mut src: ObjectReference, mut slot: Address) -> ObjectReference {
        debug_assert!(self.barrier_active());
        self.forward(unsafe { slot.load() })
    }
}

impl SSMutator {
    pub fn new(tls: *mut c_void, space: &'static CopySpace, versatile_space: &'static ImmortalSpace, los: &'static LargeObjectSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(space)),
            vs: BumpAllocator::new(tls, Some(versatile_space)),
            los: LargeObjectAllocator::new(tls, Some(los)),
            modbuf: box PLAN.modbuf_pool.spawn_local(),
            trace: box PLAN.ss_trace.values.spawn_local(),
            barrier_active: PLAN.new_barrier_active as usize,
        }
    }
    #[inline(always)]
    fn forward(&mut self, object: ObjectReference) -> ObjectReference {
        if !object.is_null() && PLAN.fromspace().in_space(object) {
            // Copy
            let mut forwarding_word = ForwardingWord::attempt_to_forward(object);
            let new_object = if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_word) {
                while ForwardingWord::state_is_being_forwarded(forwarding_word) {
                    forwarding_word = VMObjectModel::read_available_bits_word(object);
                }
                ForwardingWord::extract_forwarding_pointer(forwarding_word)
            } else {
                let new_object = VMObjectModel::mutator_copy(object, super::ss::ALLOC_SS, self);
                ForwardingWord::set_forwarding_pointer(object, new_object);
                self.trace.enqueue(new_object);
                new_object
            };
            return new_object
        }
        object
    }
    
    #[inline(always)]
    fn check_and_enqueue_reference(&mut self, object: ObjectReference) {
        if !object.is_null() && super::ss::log(object) {
            self.modbuf.enqueue(object);
        }
    }

    fn barrier_active(&self) -> bool {
        self.barrier_active != 0
    }
}

fn compare_and_swap(slot: Address, old: ObjectReference, new: ObjectReference) -> bool {
    let slot = unsafe { ::std::mem::transmute::<Address, &AtomicUsize>(slot) };
    return slot.compare_and_swap(old.to_address().as_usize(), new.to_address().as_usize(), Ordering::Relaxed) == old.to_address().as_usize()
}