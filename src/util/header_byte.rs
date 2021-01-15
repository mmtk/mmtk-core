use crate::plan::global::PlanConstraints;
use crate::util::gc_byte;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

pub const TOTAL_BITS: usize = 8;

pub struct HeaderByte {
    pub needs_unlogged_bit: bool,
    pub unlogged_bit_number: usize,
    pub unlogged_bit: u8,
    pub used_global_bits: usize,
}

impl HeaderByte {
    pub const fn new(constraints: &'static PlanConstraints) -> Self {
        let unlogged_bit_number = constraints.needs_log_bit_in_header_num;
        HeaderByte {
            needs_unlogged_bit: constraints.needs_log_bit_in_header,
            unlogged_bit_number,
            unlogged_bit: 1 << unlogged_bit_number,
            used_global_bits: TOTAL_BITS - unlogged_bit_number,
        }
    }

    pub fn mark_as_unlogged<VM: VMBinding>(&self, object: ObjectReference) {
        gc_byte::write_gc_byte::<VM>(
            object,
            gc_byte::read_gc_byte::<VM>(object) | self.unlogged_bit,
        );
    }

    pub fn mark_as_logged<VM: VMBinding>(&self, object: ObjectReference) {
        gc_byte::write_gc_byte::<VM>(
            object,
            gc_byte::read_gc_byte::<VM>(object) & !self.unlogged_bit,
        );
    }

    pub fn is_unlogged<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        (gc_byte::read_gc_byte::<VM>(object) & self.unlogged_bit) == self.unlogged_bit
    }
}
