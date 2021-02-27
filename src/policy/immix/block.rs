use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};
use crate::util::constants::*;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Block(Address);

impl Block {
    pub const LOG_PAGES_IN_BLOCK: usize = 3;
    pub const PAGES_IN_BLOCK: usize = 1 << Self::LOG_PAGES_IN_BLOCK;
    pub const LOG_BYTES_IN_BLOCK: usize = Self::LOG_PAGES_IN_BLOCK + LOG_BYTES_IN_PAGE as usize;
    pub const BYTES_IN_BLOCK: usize = 1 << Self::LOG_BYTES_IN_BLOCK;

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: 0,
        log_num_of_bits: 0,
        log_min_obj_size: Self::LOG_BYTES_IN_BLOCK,
    };

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES_IN_BLOCK));
        Self(address)
    }

    pub const fn containing(object: ObjectReference) -> Self {
        Self(object.to_address().align_down(Self::BYTES_IN_BLOCK))
    }

    pub const fn to_address(&self) -> Address {
        self.0
    }

    pub fn mark(&self) -> bool {
        side_metadata::compare_exchange_atomic(Self::MARK_TABLE, self.to_address(), 0, 1)
    }

    pub fn is_marked(&self) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.to_address()) == 1 }
    }

    pub fn clear_mark(&self) {
        side_metadata::store_atomic(Self::MARK_TABLE, self.to_address(), 0);
    }
}