use crate::{vm::{VMLocalNurseryBitSpec, VMBinding}, util::ObjectReference};
use std::sync::atomic::Ordering;

impl VMLocalNurseryBitSpec {
    pub fn is_nursery<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) -> bool {
        self.load_atomic::<VM, u8>(object, None, order) == 0
    }

    pub fn set_nursery<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, 0, None, order)
    }

    pub fn set_mature<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, 1, None, order)
    }
}
