use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::vm::VMLocalPinningBitSpec;
use std::sync::atomic::Ordering;

impl VMLocalPinningBitSpec {
    /// Pin the object
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

    pub fn is_object_pinned<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        if unsafe { self.load::<VM, u8>(object, None) == 1 } {
            return true;
        }

        false
    }
}
