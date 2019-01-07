use std::cell::UnsafeCell;

use ::policy::space::{CommonSpace, Space};
use ::util::constants::BYTES_IN_PAGE;
use ::util::heap::{FreeListPageResource, PageResource};
use ::util::{Address, ObjectReference};

const PAGE_MASK: usize = !(BYTES_IN_PAGE - 1);

fn test_mark_bit(object: ObjectReference, value: u8) -> bool {
    unimplemented!()
}

#[derive(Debug)]
pub struct LargeObjectSpace {
    common: UnsafeCell<CommonSpace<FreeListPageResource<LargeObjectSpace>>>,
    mark_state: u8,
    in_nursery_GC: bool,
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