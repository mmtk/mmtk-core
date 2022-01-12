use crate::util::conversions;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::alloc_bit;
use crate::vm::VMBinding;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::address::ByteOffset;
use crate::vm::ObjectModel;
use std::marker::PhantomData;

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

    // / Explicitly offset the cursor. We increase the cursor by `ObjectModel::get_current_size()` for each object
    // / we find. If somehow an object's actual size is not `get_current_size()`, this method can be used to offset
    // / the cursor explicitly.
    // pub fn offset_cursor(&mut self, offset: ByteOffset) {
    //     self.cursor += offset;
    // }
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
                self.cursor += VM::VMObjectModel::get_current_size(object);
                return Some(object);
            } else {
                self.cursor += VM::MIN_ALIGNMENT;
            }
        }

        None
    }
}