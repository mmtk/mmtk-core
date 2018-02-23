use std::sync::Mutex;

use ::policy::space::{Space, CommonSpace};
use ::util::heap::{PageResource, MonotonePageResource, VMRequest};
use ::util::address::Address;

use ::util::ObjectReference;
use ::util::constants::CARD_META_PAGES_PER_REGION;

use ::vm::{ObjectModel, VMObjectModel};
use ::plan::TransitiveClosure;
use ::util::header_byte;

pub struct ImmortalSpace {
    common: CommonSpace<ImmortalSpace, MonotonePageResource<ImmortalSpace>>,
    mark_state: i8,
}

const GC_MARK_BIT_MASK: i8 = 1;
const META_DATA_PAGES_PER_REGION: usize = CARD_META_PAGES_PER_REGION;

impl Space<MonotonePageResource<ImmortalSpace>> for ImmortalSpace {
    fn common(&self) -> &CommonSpace<ImmortalSpace, MonotonePageResource<ImmortalSpace>> {
        &self.common
    }
    fn common_mut(&mut self) -> &mut CommonSpace<ImmortalSpace, MonotonePageResource<ImmortalSpace>> {
        &mut self.common
    }
    fn init(&mut self) {
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };

        let common_mut = self.common_mut();
        if common_mut.vmrequest.is_discontiguous() {
            common_mut.pr = Some(MonotonePageResource::new_discontiguous(
                META_DATA_PAGES_PER_REGION));
        } else {
            common_mut.pr = Some(MonotonePageResource::new_contiguous(common_mut.start,
                                                                      common_mut.extent,
                                                                      META_DATA_PAGES_PER_REGION));
        }
        common_mut.pr.as_mut().unwrap().bind_space(me);
    }
}

impl ImmortalSpace {
    pub fn new(name: &'static str, zeroed: bool, vmrequest: VMRequest) -> Self {
        ImmortalSpace {
            common: CommonSpace::new(name, false, true, zeroed, vmrequest),
            mark_state: 0,
        }
    }

    fn test_and_mark(object: ObjectReference, value: i8) -> bool {
        let mut old_value = VMObjectModel::prepare_available_bits(object);
        let mut mark_bit = (old_value as i8) & GC_MARK_BIT_MASK;
        if mark_bit == value {
            return false;
        }
        while !VMObjectModel::attempt_available_bits(object,
                                                     old_value,
                                                     ((old_value as i8) ^ GC_MARK_BIT_MASK) as usize) {
            old_value = VMObjectModel::prepare_available_bits(object);
            mark_bit = (old_value as i8) & GC_MARK_BIT_MASK;
            if mark_bit == value {
                return false;
            }
        }
        return true;
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        if ImmortalSpace::test_and_mark(object, self.mark_state) {
            trace.process_edge(object.to_address());
        }
        return object;
    }

    pub fn initialize_header(&self, object: ObjectReference) {
        let old_value = VMObjectModel::read_available_byte(object);
        let mut new_value = (old_value & GC_MARK_BIT_MASK as u8) | self.mark_state as u8;
        if header_byte::NEEDS_UNLOGGED_BIT {
            new_value = new_value | header_byte::UNLOGGED_BIT;
        }
        VMObjectModel::write_available_byte(object, new_value);
    }

    pub fn prepare(&mut self) {
        self.mark_state = GC_MARK_BIT_MASK - self.mark_state;
    }

    pub fn release(&mut self) {}
}