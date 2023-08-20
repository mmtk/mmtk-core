use crate::{util::Address, vm::VMBinding};

use super::allocator::align_allocation_no_fill;


/// A common bump-pointer allocator shared across different allocator implementations
/// that use bump-pointer allocation.
#[repr(C)]
pub struct BumpPointer<VM: VMBinding> {
    pub cursor: Address,
    pub limit: Address,
    pub marker: std::marker::PhantomData<VM>,
}

impl<VM: VMBinding> BumpPointer<VM> {
    pub const fn new(start: Address, end: Address) -> Self {
        BumpPointer {
            cursor: start,
            limit: end,
            marker: std::marker::PhantomData,
        }
    }

    pub fn reset(&mut self, start: Address, end: Address) {
        self.cursor = start;
        self.limit = end;
    }

    pub fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        let result = align_allocation_no_fill::<VM>(self.cursor, align, offset);
        let new_cursor = result + size;
        if new_cursor <= self.limit {
            let result = self.cursor;
            self.cursor = new_cursor + size;
            return result;
        }
        unsafe { Address::zero() } 
    }
}
