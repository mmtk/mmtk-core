use std::cell::UnsafeCell;

use ::plan::TransitiveClosure;
use ::policy::space::{CommonSpace, Space};
use ::util::{Address, ObjectReference};
use ::util::constants::BYTES_IN_PAGE;
use ::util::header_byte;
use ::util::heap::{FreeListPageResource, PageResource, VMRequest};
use ::util::treadmill::TreadMill;
use ::vm::{ObjectModel, VMObjectModel};
use util::heap::layout::heap_layout::{VMMap, Mmapper};
use util::heap::HeapMeta;

const PAGE_MASK: usize = !(BYTES_IN_PAGE - 1);
const MARK_BIT: u8 = 0b01;
const NURSERY_BIT: u8 = 0b10;
const LOS_BIT_MASK: u8 = 0b11;

#[derive(Debug)]
pub struct LargeObjectSpace {
    common: UnsafeCell<CommonSpace<FreeListPageResource<LargeObjectSpace>>>,
    mark_state: u8,
    in_nursery_GC: bool,
    treadmill: TreadMill,
}

impl Space for LargeObjectSpace {
    type PR = FreeListPageResource<LargeObjectSpace>;

    fn init(&mut self, vm_map: &'static VMMap) {
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();

        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(FreeListPageResource::new_discontiguous(0, vm_map));
        } else {
            common_mut.pr = Some(FreeListPageResource::new_contiguous(me, common_mut.start, common_mut.extent, 0, vm_map));
        }

        common_mut.pr.as_mut().unwrap().bind_space(me);
    }

    fn common(&self) -> &CommonSpace<Self::PR> {
        unsafe { &*self.common.get() }
    }

    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<Self::PR> {
        &mut *self.common.get()
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        self.test_mark_bit(object, self.mark_state)
    }

    fn is_movable(&self) -> bool {
        false
    }

    fn release_multiple_pages(&mut self, start: Address) {
        self.common_mut().pr.as_mut().unwrap().release_pages(start);
    }
}

impl LargeObjectSpace {
    pub fn new(name: &'static str, zeroed: bool, vmrequest: VMRequest, vm_map: &'static VMMap, mmapper: &'static Mmapper, heap: &mut HeapMeta) -> Self {
        LargeObjectSpace {
            common: UnsafeCell::new(CommonSpace::new(name, false, false, zeroed, vmrequest, vm_map, mmapper, heap)),
            mark_state: 0,
            in_nursery_GC: false,
            treadmill: TreadMill::new()
        }
    }

    pub fn prepare(&mut self, full_heap: bool) {
        if full_heap {
            debug_assert!(self.treadmill.from_space_empty());
            self.mark_state = MARK_BIT - self.mark_state;
        }
        self.treadmill.flip(full_heap);
        self.in_nursery_GC = !full_heap;
    }

    pub fn release(&mut self, full_heap: bool) {
        self.sweep_large_pages(true);
        debug_assert!(self.treadmill.nursery_empty());
        if full_heap {
            self.sweep_large_pages(false);
        }
    }

    fn sweep_large_pages(&mut self, sweep_nursery: bool) {
        // FIXME: borrow checker fighting
        // didn't call self.release_multiple_pages
        // so the compiler knows I'm borrowing two different fields
        if sweep_nursery {
            for cell in self.treadmill.collect_nursery() {
                // println!("- cn {}", cell);
                (unsafe { &mut *self.common.get() }).pr.as_mut().unwrap().release_pages(get_super_page(cell));
            }
        } else {
            for cell in self.treadmill.collect() {
                // println!("- ts {}", cell);
                (unsafe { &mut *self.common.get() }).pr.as_mut().unwrap().release_pages(get_super_page(cell));
            }
        }
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        let nursery_object = self.is_in_nursery(object);
        if !self.in_nursery_GC || nursery_object {
            if self.test_and_mark(object, self.mark_state) {
                let cell = VMObjectModel::object_start_ref(object);
                self.treadmill.copy(cell, nursery_object);
                trace.process_node(object);
            }
        }
        return object;
    }

    pub fn initialize_header(&self, object: ObjectReference, alloc: bool) {
        let old_value = VMObjectModel::read_available_byte(object);
        let mut new_value = (old_value & (!LOS_BIT_MASK)) | self.mark_state;
        if alloc {
            new_value = new_value | NURSERY_BIT;
        }
        if header_byte::NEEDS_UNLOGGED_BIT {
            new_value = new_value | header_byte::UNLOGGED_BIT;
        }
        VMObjectModel::write_available_byte(object, new_value);
        let cell = VMObjectModel::object_start_ref(object);
        self.treadmill.add_to_treadmill(cell, alloc);
    }

    fn test_and_mark(&self, object: ObjectReference, value: u8) -> bool {
        let mask = if self.in_nursery_GC {
            LOS_BIT_MASK
        } else {
            MARK_BIT
        };
        let mut old_value = VMObjectModel::prepare_available_bits(object);
        let mut mark_bit = (old_value as u8) & mask;
        if mark_bit == value {
            return false;
        }
        while !VMObjectModel::attempt_available_bits(
            object,
            old_value,
            old_value & (!LOS_BIT_MASK as usize) | value as usize) {
            old_value = VMObjectModel::prepare_available_bits(object);
            mark_bit = (old_value as u8) & mask;
            if mark_bit == value {
                return false;
            }
        }
        return true;
    }

    fn test_mark_bit(&self, object: ObjectReference, value: u8) -> bool {
        VMObjectModel::read_available_byte(object) & MARK_BIT == value
    }

    fn is_in_nursery(&self, object: ObjectReference) -> bool {
        VMObjectModel::read_available_byte(object) & NURSERY_BIT == NURSERY_BIT
    }
}

fn get_super_page(cell: Address) -> Address {
    unsafe { Address::from_usize(cell.as_usize() & PAGE_MASK) }
}