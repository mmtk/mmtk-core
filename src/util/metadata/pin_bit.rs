#[cfg(feature = "object_pinning")]
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMLocalPinningBitSpec;
use std::sync::atomic::Ordering;

impl VMLocalPinningBitSpec {
    /// Pin an object by setting the pinning bit to 1.
    /// Return true if the object is pinned in this operation.
    pub fn pin_object<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        let res = self.compare_exchange_metadata::<VM, u8>(
            object,
            0,
            1,
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        res.is_ok()
    }

    /// Unpin an object by clearing the pinning bit to 0.
    /// Return true if the object is unpinned in this operation.
    pub fn unpin_object<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        let res = self.compare_exchange_metadata::<VM, u8>(
            object,
            1,
            0,
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        res.is_ok()
    }

    /// Check if an object is pinned.
    pub fn is_object_pinned<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        if unsafe { self.load::<VM, u8>(object, None) == 1 } {
            return true;
        }

        false
    }
}

/// Bulk zero the pin bit.
#[cfg(feature = "object_pinning")]
pub fn bzero_pin_bit<VM: VMBinding>(start: Address, size: usize) {
    use crate::util::metadata::MetadataSpec;
    use crate::vm::object_model::ObjectModel;

    if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC {
        // We zero all the pin bits for the address range.
        side.bzero_metadata(start, size);
    } else {
        // If the pin bit is not in side metadata, we cannot bulk zero.
        // We will probably have to clear it for new objects, which means
        // that we do not need to clear it at sweeping.
        unimplemented!("We cannot bulk zero pin bit.")
    }
}
