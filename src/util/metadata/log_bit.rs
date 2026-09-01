use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::vm::VMGlobalLogBitSpec;
use std::sync::atomic::Ordering;

use super::MetadataSpec;

/// The value stored in the log bit/byte indicating that the object or field is unlogged, i.e. it
/// has not yet been recorded in the remembered set and the write barrier should still take its slow path.
pub const UNLOGGED_VALUE: u8 = 0b1;

/// The value stored in the log bit/byte indicating that the object or field is logged, i.e. it
/// has already been recorded in the remembered set and the write barrier can skip its slow path.
pub const LOGGED_VALUE: u8 = 0b0;

impl VMGlobalLogBitSpec {
    /// Clear the unlog bit to log object (0 means logged)
    pub fn clear<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, LOGGED_VALUE, None, order)
    }

    /// Bulk clear the unlog bits for a memory range
    pub fn bulk_mark_as_logged(&self, start: Address, size: usize) {
        if let MetadataSpec::OnSide(side) = self.as_spec() {
            side.bzero_metadata(start, size);
        }
    }

    /// Mark the log bit as unlogged (1 means unlogged)
    pub fn mark_as_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, UNLOGGED_VALUE, None, order)
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
                spec.set_raw_byte_atomic(object.to_raw_address(), order)
            },
        }
    }

    /// Check if the log bit represents the unlogged state.
    pub fn is_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) -> bool {
        self.load_atomic::<VM, u8>(object, None, order) == UNLOGGED_VALUE
    }
}

impl MetadataSpec {
    /// Mark the log bit as unlogged (1 means unlogged)
    pub fn mark_as_unlogged<VM: VMBinding>(&self, object: ObjectReference, order: Ordering) {
        self.store_atomic::<VM, u8>(object, UNLOGGED_VALUE, None, order)
    }
}

/// This specifies what to do to the global side unlog bits in various functions or work packets.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnlogBitsOperation {
    /// Do nothing.
    NoOp,
    /// Bulk set unlog bits to all 1s.
    BulkSet,
    /// Bulk clear unlog bits to all 0s.
    BulkClear,
}

impl UnlogBitsOperation {
    /// Run the specified operation on the address range from `start` to `start + size`.
    pub(crate) fn execute<VM: VMBinding>(&self, start: Address, size: usize) {
        if let MetadataSpec::OnSide(ref unlog_bits) = *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC {
            match self {
                UnlogBitsOperation::NoOp => {}
                UnlogBitsOperation::BulkSet => {
                    unlog_bits.bset_metadata(start, size);
                }
                UnlogBitsOperation::BulkClear => {
                    unlog_bits.bzero_metadata(start, size);
                }
            }
        }
    }
}
