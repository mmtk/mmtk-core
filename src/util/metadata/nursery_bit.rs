use crate::{vm::{VMLocalNurseryBitSpec, VMBinding}, util::ObjectReference};
use std::sync::atomic::Ordering;

impl VMLocalNurseryBitSpec {
    pub fn is_nursery<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) -> bool {
        self.load_atomic::<VM, u8>(object, None, order) == 1
    }

    pub fn set<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, 1, None, order)
    }

    pub fn clear<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, 0, None, order)
    }
}