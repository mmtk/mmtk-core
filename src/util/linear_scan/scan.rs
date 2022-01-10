use super::callback::LinearScanCallback;
use crate::util::conversions;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::alloc_bit;
use crate::vm::VMBinding;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;

use std::marker::PhantomData;

/// The caller needs to ensure that memory are accessible between `start` and `end`,
/// and we have valid alloc bit mapping for the address range as well.
pub fn scan_region<VM: VMBinding, C: LinearScanCallback, const ATOMIC_LOAD_ALLOC_BIT: bool>(start: Address, end: Address, callback: &mut C) {
    let mut address = start;
    let mut page = conversions::page_align_down(start);

    while address < end {
        if address - page >= BYTES_IN_PAGE {
            callback.on_page(page);
            page = conversions::page_align_down(address);
        }

        let is_object = if ATOMIC_LOAD_ALLOC_BIT {
            alloc_bit::is_alloced_object(address)
        } else {
            unsafe { alloc_bit::is_alloced_object_unsafe(address) }
        };

        if is_object {
            let object = unsafe { address.to_object_reference() };
            let bytes = callback.on_object(object);
            address += bytes;
        } else {
            address += VM::MIN_ALIGNMENT;
        }
    }
}

pub struct LinearScanIterator<VM: VMBinding, const ATOMIC_LOAD_ALLOC_BIT: bool> {
    start: Address,
    end: Address,
    cursor: Address,
    _p: PhantomData<VM>
}

impl<VM: VMBinding, const ATOMIC_LOAD_ALLOC_BIT: bool> LinearScanIterator<VM, ATOMIC_LOAD_ALLOC_BIT> {
    pub fn new(start: Address, end: Address) -> Self {
        debug_assert!(start < end);
        LinearScanIterator {
            start,
            end,
            cursor: start,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding, const ATOMIC_LOAD_ALLOC_BIT: bool> std::iter::Iterator for LinearScanIterator<VM, ATOMIC_LOAD_ALLOC_BIT> {
    type Item = ObjectReference;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        while self.cursor < self.end {
            let is_object = if ATOMIC_LOAD_ALLOC_BIT {
                alloc_bit::is_alloced_object(self.cursor)
            } else {
                unsafe { alloc_bit::is_alloced_object_unsafe(self.cursor) }
            };

            if is_object {
                let object = unsafe { self.cursor.to_object_reference() };
                let bytes = VM::VMObjectModel::get_current_size(object);
                self.cursor += bytes;
                return Some(object);
            } else {
                self.cursor += VM::MIN_ALIGNMENT;
            }
        }

        None
    }
}