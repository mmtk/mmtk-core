use std::cell::UnsafeCell;

use ::policy::space::{CommonSpace, Space};
use ::util::{Address, ObjectReference};
use ::util::constants::BYTES_IN_PAGE;
use ::util::heap::{FreeListPageResource, PageResource};
use ::util::treadmill::TreadMill;
use ::plan::TransitiveClosure;

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

    fn init(&mut self) {
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();

        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(FreeListPageResource::new_discontiguous(0));
        } else {
            common_mut.pr = Some(FreeListPageResource::new_contiguous(me, common_mut.start, common_mut.extent, 0));
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
        test_mark_bit(object, self.mark_state)
    }

    fn is_movable(&self) -> bool {
        false
    }

    fn release_multiple_pages(&mut self, start: Address) {
        self.common_mut().pr.as_mut().unwrap().release_pages(start);
    }
}

impl LargeObjectSpace {
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
            for cell in self.treadmill.iter_nursery() {
                (unsafe {&mut *self.common.get() }).pr.as_mut().unwrap().release_pages(get_super_page(cell));
            }
        } else {
            for cell in self.treadmill.iter() {
                (unsafe {&mut *self.common.get() }).pr.as_mut().unwrap().release_pages(get_super_page(cell));
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
                // FIXME self.treadmill.copy(node, nursery_object);
                trace.process_node(object);
            }
        }
        return object;
    }

    fn test_and_mark(&self, object: ObjectReference, value: u8) -> bool {
        unimplemented!()
    }

    fn test_mark_bit(&self, object: ObjectReference, value: u8) -> bool {
        unimplemented!()
    }

    fn is_in_nursery(&self, object: ObjectReference) -> bool {
        unimplemented!()
    }
}

fn get_super_page(cell: &Address) -> Address {
    unimplemented!()
}