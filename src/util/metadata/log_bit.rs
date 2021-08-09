use crate::util::metadata::*;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMGlobalLogBitSpec;
use std::sync::atomic::Ordering;

impl VMGlobalLogBitSpec {
    /// Mark the log bit as unlogged (1 means unlogged)
    pub fn mark_as_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        store_metadata::<VM>(&self, object, 1, None, Some(order))
    }
}
