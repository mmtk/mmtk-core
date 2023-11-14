use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMGlobalLogBitSpec;
use std::sync::atomic::Ordering;

use super::MetadataSpec;

impl VMGlobalLogBitSpec {
    /// Mark the log bit as unlogged (1 means unlogged)
    pub fn mark_as_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, 1, None, order)
    }

    /// Mark the entire byte as unlogged if the log bit is in the side metadata. As it marks the entire byte,
    /// it may unlog adjacent objects. This method should only be used
    /// when adjacent objects are also in the mature space, and there is no harm if we also unlog them.
    /// This method is meant to be an optimization, and can always be replaced with `mark_as_unlogged`.
    pub fn mark_byte_as_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        match self.as_spec() {
            // If the log bit is in the header, there is nothing we can do. We just call `mark_as_unlogged`.
            MetadataSpec::InHeader(_) => self.mark_as_unlogged::<VM>(object, order),
            // If the log bit is in the side metadata, we can simply set the entire byte to 0xff. Because we
            // know we are setting log bit for mature space, and every object in the space should have log
            // bit as 1.
            MetadataSpec::OnSide(spec) => unsafe {
                spec.set_raw_byte_atomic(object.to_address::<VM>(), order)
            },
        }
    }

    /// Check if the log bit represents the unlogged state (the bit is 1).
    pub fn is_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) -> bool {
        self.load_atomic::<VM, u8>(object, None, order) == 1
    }
}
