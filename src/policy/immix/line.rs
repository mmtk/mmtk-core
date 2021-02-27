use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};



#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Line(Address);

impl Line {
    pub const LOG_BYTES: usize = 8;
    pub const BYTES: usize = 1 << Self::LOG_BYTES;

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: 0,
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    pub const fn containing(object: ObjectReference) -> Self {
        Self(object.to_address().align_down(Self::BYTES))
    }

    pub const fn start(&self) -> Address {
        self.0
    }

    pub const fn end(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    #[inline]
    pub fn attempt_mark(&self) -> bool {
        side_metadata::compare_exchange_atomic(Self::MARK_TABLE, self.start(), 0, 1)
    }

    #[inline]
    pub fn mark(&self) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), 1); }
    }

    #[inline]
    pub fn is_marked(&self) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) == 1 }
    }

    #[inline]
    pub fn clear_mark(&self) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), 0); }
    }
}
