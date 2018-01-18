use super::space::default;

use std::sync::Mutex;

use ::policy::space::Space;
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::address::Address;

use ::util::ObjectReference;

use ::vm::{ObjectModel, VMObjectModel};
use ::plan::TransitiveClosure;
use ::util::header_byte;

pub struct ImmortalSpace {
    pr: Mutex<MonotonePageResource>,
}

const GC_MARK_BIT_MASK: i8 = 1;
const MARK_STATE: i8 = 0;

impl Space for ImmortalSpace {
    fn init(&self, heap_size: usize) {
        default::init(&self.pr, heap_size);
    }

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        default::acquire(&self.pr, thread_id, size)
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        default::in_space(&self.pr, object)
    }
}

impl ImmortalSpace {
    pub fn new() -> Self {
        ImmortalSpace {
            pr: Mutex::new(MonotonePageResource::new()),
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
        if ImmortalSpace::test_and_mark(object, MARK_STATE) {
            trace.process_edge(object.to_address());
        }
        return object;
    }

    pub fn initialize_header(object: ObjectReference) {
        let old_value = VMObjectModel::read_available_byte(object);
        let mut new_value = (old_value & GC_MARK_BIT_MASK as u8) | MARK_STATE as u8;
        if header_byte::NEEDS_UNLOGGED_BIT {
            new_value = new_value | header_byte::UNLOGGED_BIT;
        }
        VMObjectModel::write_available_byte(object, new_value);
    }
}