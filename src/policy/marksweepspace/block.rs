use crate::util::{Address, alloc::free_list_allocator::BYTES_IN_BLOCK};

pub struct Block(Address);

impl Block {
    /// Align the address to a block boundary.
    pub const fn align(address: Address) -> Address {
        address.align_down(BYTES_IN_BLOCK)
    }
}
pub enum BlockState {
    /// the block is not allocated.
    Unallocated,
    /// the block is allocated but not marked.
    Unmarked,
    /// the block is allocated and marked.
    Marked,
    /// the block is marked as reusable.
    Reusable { unavailable_lines: u8 },
}