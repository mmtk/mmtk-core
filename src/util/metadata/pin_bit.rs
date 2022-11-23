use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMLocalPinningBitSpec;
use std::sync::atomic::Ordering;

impl VMLocalPinningBitSpec {
    /// Pin the object
    pub fn pin_object<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        debug_assert!(
            !crate::util::object_forwarding::is_forwarded_or_being_forwarded::<VM>(object),
            "Object to be unpinned should not be forwarded or being forwarded."
        );

        let res = self.compare_exchange_metadata::<VM, u8>(
            object,
            0,
            1,
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        match res {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    pub fn unpin_object<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        debug_assert!(
            !crate::util::object_forwarding::is_forwarded_or_being_forwarded::<VM>(object),
            "Object to be unpinned should not be forwarded or being forwarded."
        );

        let res = self.compare_exchange_metadata::<VM, u8>(
            object,
            1,
            0,
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        match res {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    pub fn is_object_pinned<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        if unsafe { self.load::<VM, u8>(object, None) == 1 } {
            return true;
        }

        return false;
    }
}
