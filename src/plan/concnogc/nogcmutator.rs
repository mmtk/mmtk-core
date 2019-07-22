use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::util::heap::MonotonePageResource;
use ::plan::plan;
use super::PLAN;
use ::vm::*;
use util::queue::LocalQueue;
use super::nogc::{HEAD, set_next};
use std::sync::atomic::{Ordering, AtomicUsize};

use libc::c_void;

#[repr(C)]
pub struct NoGCMutator {
    nogc: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    modbuf: Box<LocalQueue<'static, ObjectReference>>,
    barrier_active: bool,
}

lazy_static! {
    static ref LOCK: ::std::sync::Mutex<()> = ::std::sync::Mutex::new(());
}

impl MutatorContext for NoGCMutator {
    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, _primary: bool) {
        if super::VERBOSE {
            println!("Mutator {:?}", phase);
        }
        match phase {
            &Phase::SetBarrierActive => {
                self.barrier_active = true;
            }
            &Phase::ClearBarrierActive => {
                self.barrier_active = false;
            }
            &Phase::FlushMutator => {
                self.nogc.reset();
                self.flush();
            }
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.nogc.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.nogc.reset();
                self.flush_remembered_sets();
            }
            &Phase::Release => {
                self.nogc.reset();
                self.flush();
                // debug_assert!(self.modbuf.is_empty());
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, _allocator: AllocationType) -> Address {
        self.nogc.alloc(size, align, offset)

    }

    fn alloc_slow(&mut self, _size: usize, _align: usize, _offset: isize, _allocator: AllocationType) -> Address {
        unreachable!()
    }

    fn post_alloc(&mut self, obj: ObjectReference, _type_refer: ObjectReference, _bytes: usize, _allocator: AllocationType) {
        // Add to linked list
        let mut head = HEAD.lock().unwrap();
        set_next(obj, *head);
        *head = obj;
        // Mark object
        let mark_slot = obj.to_address() + (VMObjectModel::GC_HEADER_OFFSET() + 2isize);
        unsafe { mark_slot.store(PLAN.mark_state as u16) };
    }

    fn object_reference_write_slow(&mut self, _src: ObjectReference, slot: Address, _value: ObjectReference) {
        if self.barrier_active {
            self.check_and_enqueue_reference(unsafe { slot.load::<ObjectReference>() });
        }
    }

    fn object_reference_try_compare_and_swap_slow(&mut self, _src: ObjectReference, slot: Address, old: ObjectReference, new: ObjectReference) -> bool {
        if self.barrier_active {
            self.check_and_enqueue_reference(old);
        }
        let slot = unsafe { ::std::mem::transmute::<Address, &AtomicUsize>(slot) };
        return slot.compare_and_swap(old.to_address().as_usize(), new.to_address().as_usize(), Ordering::Relaxed) == old.to_address().as_usize()
    }

    fn java_lang_reference_read_slow(&mut self, obj: ObjectReference) -> ObjectReference {
        if self.barrier_active {
            self.check_and_enqueue_reference(obj);
        }
        obj
    }

    fn get_tls(&self) -> *mut c_void {
        self.nogc.tls
    }

    fn flush_remembered_sets(&mut self) {
        self.modbuf.flush();
    } 
}

impl NoGCMutator {
    pub fn new(tls: *mut c_void, space: &'static ImmortalSpace) -> Self {
        NoGCMutator {
            nogc: BumpAllocator::new(tls, Some(space)),
            modbuf: box PLAN.modbuf_pool.spawn_local(),
            barrier_active: PLAN.new_barrier_active,
        }
    }
    #[inline(always)]
    fn check_and_enqueue_reference(&mut self, object: ObjectReference) {
        if object.is_null() {
            return;
        }
        let log_word = object.to_address() + VMObjectModel::GC_HEADER_OFFSET();
        let log_state = PLAN.log_state;
        if unsafe { log_word.load::<u16>() } != log_state {
            unsafe { log_word.store(log_state as u16) };
            self.modbuf.enqueue(object);
        }
    }
}