use mmtk::util::constants::LOG_BYTES_IN_ADDRESS;
use mmtk::{
    util::{Address, ObjectReference},
    vm::slot::{MemorySlice, SimpleSlot},
};

// A binding may implement their own type of slots.
// See https://github.com/mmtk/mmtk-core/blob/master/src/vm/tests/mock_tests/mock_test_slots.rs for different kinds of slots.
pub type DummyVMSlot = SimpleSlot;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DummyVMMemorySlice(*mut [ObjectReference]);

unsafe impl Send for DummyVMMemorySlice {}

impl MemorySlice for DummyVMMemorySlice {
    type SlotType = DummyVMSlot;
    type SlotIterator = DummyVMMemorySliceIterator;

    fn iter_slots(&self) -> Self::SlotIterator {
        DummyVMMemorySliceIterator {
            cursor: unsafe { (*self.0).as_mut_ptr_range().start },
            limit: unsafe { (*self.0).as_mut_ptr_range().end },
        }
    }

    fn object(&self) -> Option<ObjectReference> {
        None
    }

    fn start(&self) -> Address {
        Address::from_ptr(unsafe { (*self.0).as_ptr_range().start })
    }

    fn bytes(&self) -> usize {
        unsafe { std::mem::size_of_val(&*self.0) }
    }

    fn copy(src: &Self, tgt: &Self) {
        debug_assert_eq!(src.bytes(), tgt.bytes());
        debug_assert_eq!(
            src.bytes() & ((1 << LOG_BYTES_IN_ADDRESS) - 1),
            0,
            "bytes are not a multiple of words"
        );
        // Raw memory copy
        unsafe {
            let words = tgt.bytes() >> LOG_BYTES_IN_ADDRESS;
            let src = src.start().to_ptr::<usize>();
            let tgt = tgt.start().to_mut_ptr::<usize>();
            std::ptr::copy(src, tgt, words)
        }
    }
}

pub struct DummyVMMemorySliceIterator {
    cursor: *mut ObjectReference,
    limit: *mut ObjectReference,
}

impl Iterator for DummyVMMemorySliceIterator {
    type Item = DummyVMSlot;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.limit {
            None
        } else {
            let edge = self.cursor;
            self.cursor = unsafe { self.cursor.add(1) };
            Some(SimpleSlot::from_address(Address::from_ptr(edge)))
        }
    }
}
