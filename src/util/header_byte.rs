use crate::plan::SelectedConstraints;
use crate::util::object_gc_stats::GCByte;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

pub const TOTAL_BITS: usize = 8;
pub const NEEDS_UNLOGGED_BIT: bool = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER;
pub const UNLOGGED_BIT_NUMBER: usize = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER_NUM;
pub const UNLOGGED_BIT: u8 = 1 << UNLOGGED_BIT_NUMBER;
pub const USED_GLOBAL_BITS: usize = TOTAL_BITS - UNLOGGED_BIT_NUMBER;

pub fn mark_as_unlogged<VM: VMBinding>(object: ObjectReference) {
    GCByte::write::<VM>(object, GCByte::read::<VM>(object) | UNLOGGED_BIT);
}

pub fn mark_as_logged<VM: VMBinding>(object: ObjectReference) {
    GCByte::write::<VM>(object, GCByte::read::<VM>(object) & !UNLOGGED_BIT);
}

pub fn is_unlogged<VM: VMBinding>(object: ObjectReference) -> bool {
    (GCByte::read::<VM>(object) & UNLOGGED_BIT) == UNLOGGED_BIT
}
