use ::policy::space::Space;
use ::util::address::Address;

use ::util::ObjectReference;

use ::vm::{ObjectModel, VMObjectModel};
use ::plan::TransitiveClosure;
use ::util::header_byte;

pub struct BootSpace {
    start: usize,
    end: usize,
    mark_state: i8,
}

const GC_MARK_BIT_MASK: i8 = 1;

impl Space for BootSpace {
    fn init(&self, heap_size: usize){

    }

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        unimplemented!()
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let addr = object.to_address().as_usize();
        addr >= self.start && addr <= self.end
    }
}

impl BootSpace {
    pub fn new() -> Self {
        BootSpace {
            start: 0x60000000,
            end: 0x67ffffff,
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
        if BootSpace::test_and_mark(object, self.mark_state) {
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