use ::plan::SelectedConstraints;
use ::util::{Address, ObjectReference};
use ::vm::{ObjectModel, VMObjectModel};

pub const TOTAL_BITS: usize = 8;
pub const NEEDS_UNLOGGED_BIT: bool = SelectedConstraints::NEEDS_LOG_BIT_IN_HEADER;
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

pub fn attempt_unlog(object: ObjectReference) -> bool {
    loop {
        let old = VMObjectModel::prepare_available_bits(object);
        if (old & (UNLOGGED_BIT as usize)) == (UNLOGGED_BIT as usize) {
            return false; // Already unlogged
        }
        let new = old | (UNLOGGED_BIT as usize);
        if VMObjectModel::attempt_available_bits(object, old, new) {
            return true
        }
    }
}

pub fn attempt_log(object: ObjectReference) -> bool {
    loop {
        let old = VMObjectModel::prepare_available_bits(object);
        if (old & (UNLOGGED_BIT as usize)) == 0usize {
            return false; // Already logged
        }
        let new = old & !(UNLOGGED_BIT as usize);
        if VMObjectModel::attempt_available_bits(object, old, new) {
            return true
        }
    }
}