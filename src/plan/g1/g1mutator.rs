use policy::region::*;
use policy::immortalspace::ImmortalSpace;
use util::alloc::{BumpAllocator, RegionAllocator};
use plan::mutator_context::MutatorContext;
use plan::Phase;
use util::{Address, ObjectReference};
use util::alloc::Allocator;
use plan::Allocator as AllocationType;
use plan::plan;
use vm::*;
use util::heap::{MonotonePageResource};
use plan::g1::{PLAN, VERBOSE};
use util::alloc::LargeObjectAllocator;
use policy::largeobjectspace::LargeObjectSpace;
use util::queue::LocalQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use libc::c_void;

#[repr(C)]
pub struct G1Mutator {
    rs: RegionAllocator,
    los: LargeObjectAllocator,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    modbuf: Box<LocalQueue<'static, ObjectReference>>,
    dirty_card_quene: Box<Vec<Card>>,
    barrier_active: usize,
}

impl MutatorContext for G1Mutator {
    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, _primary: bool) {
        if VERBOSE {
            // println!("Mutator {:?}", phase);
        }
        match phase {
            &Phase::SetBarrierActive => {
                self.flush();
                self.barrier_active = 1;
            }
            &Phase::ClearBarrierActive => {
                self.barrier_active = 0;
            }
            &Phase::FlushMutator => {
                self.flush();
            }
            &Phase::FinalClosure => {
                self.flush();
            }
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.rs.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.rs.adjust_tlab_size();
                self.rs.reset();
                self.vs.reset();
            }
            &Phase::Release => {
                self.rs.reset();
                self.vs.reset();
            }
            &Phase::Complete => {
                self.rs.reset();
                self.vs.reset();
                self.modbuf.reset();
                self.dirty_card_quene.clear();
            }
            &Phase::RefineCards => {
                self.dirty_card_quene.clear();
            }
            &Phase::EvacuatePrepare => {
                self.rs.reset();
                self.vs.reset();
            }
            &Phase::EvacuateRelease => {
                self.rs.reset();
                self.vs.reset();
            }
            &Phase::ValidatePrepare => {
                self.rs.reset();
                self.vs.reset();
            }
            &Phase::ValidateRelease => {
                self.rs.reset();
                self.vs.reset();
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.rs.space as *const _ == &PLAN.region_space as *const _,
                      "regionallocator {:?} holds wrong space, rs.space: {:?}, region_space: {:?}",
                      self as *const _,
                      self.rs.get_space().unwrap() as *const _,
                      &PLAN.region_space as *const _);
                      unimplemented!()
        // match allocator {
        //     AllocationType::Default => self.rs.alloc(size, align, offset),
        //     AllocationType::Los => self.los.alloc(size, align, offset),
        //     _ => self.vs.alloc(size, align, offset),
        // }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.rs.get_space().unwrap() as *const _ == &PLAN.region_space as *const _,
                      "regionallocator {:?} holds wrong space, rs.space: {:?}, region_space: {:?}",
                      self as *const _,
                      self.rs.get_space().unwrap() as *const _,
                      &PLAN.region_space as *const _);
        match allocator {
            AllocationType::Default => self.rs.alloc_slow(size, align, offset),
            AllocationType::Los => self.los.alloc(size, align, offset),
            _ => self.vs.alloc_slow(size, align, offset),
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, _type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.rs.get_space().unwrap() as *const _ == &PLAN.region_space as *const _);
        match allocator {
            AllocationType::Default => {
                // println!("Alloc {:?} end {:?} {:?}", refer, VMObjectModel::object_start_ref(refer) + bytes, VMObjectModel::get_object_end_address(refer));
                PLAN.region_space.initialize_header(refer, bytes, true, false, true);
            }
            AllocationType::Los => {
                PLAN.los.initialize_header(refer, true);
            }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                PLAN.versatile_space.initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> *mut c_void {
        debug_assert!(self.rs.tls == self.vs.tls);
        self.rs.tls
    }

    fn object_reference_write_slow(&mut self, src: ObjectReference, slot: Address, value: ObjectReference) {
        if super::ENABLE_CONCURRENT_MARKING && self.barrier_active() {
            let old = unsafe { slot.load::<ObjectReference>() };
            self.check_and_enqueue_reference(old);
        }

        unsafe { slot.store(value) }

        self.card_marking_barrier(src, slot);
    }

    fn object_reference_try_compare_and_swap_slow(&mut self, src: ObjectReference, slot: Address, old: ObjectReference, new: ObjectReference) -> bool {
        if super::ENABLE_CONCURRENT_MARKING && self.barrier_active() {
            self.check_and_enqueue_reference(old);
        }

        let aslot = unsafe { ::std::mem::transmute::<Address, &AtomicUsize>(slot) };
        let result = aslot.compare_and_swap(old.to_address().as_usize(), new.to_address().as_usize(), Ordering::Relaxed) == old.to_address().as_usize();

        self.card_marking_barrier(src, slot);

        result
    }

    fn java_lang_reference_read_slow(&mut self, obj: ObjectReference) -> ObjectReference {
        debug_assert!(self.barrier_active());
        if super::ENABLE_CONCURRENT_MARKING {
            self.check_and_enqueue_reference(obj);
        }
        obj
    }

    fn flush_remembered_sets(&mut self) {
        self.rs.reset();
        self.vs.reset();
        self.modbuf.flush();
    }
}

impl G1Mutator {
    pub fn new(tls: *mut c_void, space: &'static mut RegionSpace, los: &'static LargeObjectSpace, versatile_space: &'static ImmortalSpace) -> Self {
        G1Mutator {
            rs: RegionAllocator::new(tls, space),
            los: LargeObjectAllocator::new(tls, Some(los)),
            vs: BumpAllocator::new(tls, Some(versatile_space)),
            modbuf: box PLAN.modbuf_pool.spawn_local(),
            dirty_card_quene: box Vec::with_capacity(super::DIRTY_CARD_QUEUE_SIZE),
            barrier_active: PLAN.new_barrier_active as usize,
        }
    }
    
    #[inline(always)]
    fn check_and_enqueue_reference(&mut self, object: ObjectReference) {
        if !object.is_null() && ::util::header_byte::attempt_unlog(object) {
            self.modbuf.enqueue(object);
        }
    }

    #[inline(always)]
    fn card_marking_barrier(&mut self, src: ObjectReference, _slot: Address) {
        if !super::ENABLE_REMEMBERED_SETS {
            return // we don't need remsets
        }
        let card = Card::of(src);
        
        if card.get_state() == CardState::NotDirty {
            card.set_state(CardState::Dirty);
            if super::ENABLE_CONCURRENT_REFINEMENT {
                self.rs_enquene(card);
            }
        }
    }

    fn flush_dirty_card_queue(&mut self) {
        let mut b = box Vec::with_capacity(super::DIRTY_CARD_QUEUE_SIZE);
        ::std::mem::swap(&mut b, &mut self.dirty_card_quene);
        super::concurrent_refine::enquene(b);
    }

    fn rs_enquene(&mut self, card: Card) {
        debug_assert!(self.dirty_card_quene.len() < super::DIRTY_CARD_QUEUE_SIZE);
        self.dirty_card_quene.push(card);
        if self.dirty_card_quene.len() == super::DIRTY_CARD_QUEUE_SIZE {
            self.flush_dirty_card_queue()
        }
    }

    #[inline(always)]
    fn barrier_active(&self) -> bool {
        self.barrier_active != 0
    }
}