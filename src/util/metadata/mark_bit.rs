use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMLocalMarkBitSpec;
use std::sync::atomic::Ordering;

impl VMLocalMarkBitSpec {
    /// Set the mark bit for the object to 1
    pub fn mark<VM: VMBinding>(&self, object: ObjectReference, ordering: Ordering) {
        self.store_atomic::<VM, u8>(object, 1, None, ordering);
    }

    /// Test if the mark bit for the object is set (1)
    pub fn is_marked<VM: VMBinding>(&self, object: ObjectReference, ordering: Ordering) -> bool {
        self.load_atomic::<VM, u8>(object, None, ordering) == 1
    }
}
