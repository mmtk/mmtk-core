use ::plan::SelectedConstraints;
use ::util::{Address, ObjectReference};
use ::vm::{ObjectModel, VMObjectModel};

pub const TOTAL_BITS: usize = 8;
pub const UNLOGGED_BIT_NUMBER: usize = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER_NUM;
pub const UNLOGGED_BIT: u8 = 1 << UNLOGGED_BIT_NUMBER;
pub const USED_GLOBAL_BITS: usize = TOTAL_BITS - UNLOGGED_BIT_NUMBER;

pub fn mark_as_unlogged(object: ObjectReference) {
    let value = VMObjectModel::read_available_byte(object);
    VMObjectModel::write_available_byte(object, value | UNLOGGED_BIT);
}

pub fn mark_as_logged(object: ObjectReference) {
    let value = VMObjectModel::read_available_byte(object);
    VMObjectModel::write_available_byte(object, value & !UNLOGGED_BIT);
}

pub fn is_unlogged(object: ObjectReference) -> bool {
    let value = VMObjectModel::read_available_byte(object);
    (value & UNLOGGED_BIT) == UNLOGGED_BIT
}