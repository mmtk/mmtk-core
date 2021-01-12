// use crate::plan::SelectedConstraints;
use crate::util::gc_byte;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::plan::global::PlanTypes;
use crate::plan::global::PlanConstraints;

pub const TOTAL_BITS: usize = 8;
// pub const NEEDS_UNLOGGED_BIT: bool = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER;
// pub const UNLOGGED_BIT_NUMBER: usize = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER_NUM;
// pub const UNLOGGED_BIT: u8 = 1 << UNLOGGED_BIT_NUMBER;
// pub const USED_GLOBAL_BITS: usize = TOTAL_BITS - UNLOGGED_BIT_NUMBER;

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
}

const fn unlogged_bit<P: PlanTypes>() -> u8 {
    1 << P::NEEDS_LOG_BIT_IN_HEADER_NUM
}

pub fn mark_as_unlogged<VM: VMBinding, P: PlanTypes>(object: ObjectReference) {
    gc_byte::write_gc_byte::<VM>(object, gc_byte::read_gc_byte::<VM>(object) | unlogged_bit::<P>());
}

pub fn mark_as_logged<VM: VMBinding, P: PlanTypes>(object: ObjectReference) {
    gc_byte::write_gc_byte::<VM>(object, gc_byte::read_gc_byte::<VM>(object) & !unlogged_bit::<P>());
}

pub fn is_unlogged<VM: VMBinding, P: PlanTypes>(object: ObjectReference) -> bool {
    (gc_byte::read_gc_byte::<VM>(object) & unlogged_bit::<P>()) == unlogged_bit::<P>()
}
