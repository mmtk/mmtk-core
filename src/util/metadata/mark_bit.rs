use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
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

/// This provides an abstraction of the mark bit. It abstracts over the difference between side mark bits and in-header mark bits,
/// and provides efficient implementation for each case.
///
/// The key difference between side and in-header mark bit is what the mark state represents.
///
/// * Side mark bit
///   We always use 1 for the marked state. So we do not need to set mark bit for new objects (mark bit 0). In each GC, we do bulk zeroing
///   to reset the mark bit to 0 before tracing or after tracing. During tracing, we mark objects with the state 1 as usual.
/// * In-header mark bit
///   We flip the mark state in every GC. For example, if 1 means marked in the current GC, 1 will mean unmarked in the next GC.
///   With this approach, we do not need to reset mark bit for each object, as the value represents unmarked in the next GC.
///   However, with in-header mark bit, we have to set mark bit for newly allocated objects.
///
/// A policy could use this struct instead of the raw mark bit. It has to call all the methods prefixed with `on_`
/// such as `on_object_metadata_initialization()`, `on_global_prepare()`, `on_block_prepare()`, and `on_global_release()`.
// TODO: Currently only ImmortalSpace uses this struct. Any policy that needs mark bit can use this (immix, mark compact, mark sweep).
// We should do some refactoring for other policies as well.
pub struct MarkState {
    /// This value represents the marked state. If the mark bit is this value, the object is considered as marked.
    /// If the mark bit is on side, we always use 1 as the marked state. We do bulk zeroing to reset mark bits before GCs
    /// If the mark bit is in header, we flip the marked state in every GC, so we do not need to reset the mark bit for live objects.
    state: u8,
}

impl MarkState {
    pub fn new() -> Self {
        Self { state: 1 }
    }

    fn unmarked_state(&self) -> u8 {
        self.state ^ 1
    }

    /// Check if the object is marked
    pub fn is_marked<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        let state = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        state == self.state
    }

    /// Unconditionally mark object. Note that users of this function should ensure that either
    /// there is no possible race (only one thread can set the mark bit) or the races are benign
    /// (it doesn't matter which thread sets the mark bit).
    pub fn mark<VM: VMBinding>(&self, object: ObjectReference) {
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
            object,
            self.state,
            None,
            Ordering::SeqCst,
        );
    }

    /// Attempt to mark an object. If the object is marked by this invocation, return true.
    /// Otherwise return false -- the object was marked by others.
    pub fn test_and_mark<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        loop {
            let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            if old_value == self.state {
                return false;
            }

            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    self.state,
                    None,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        true
    }

    /// This has to be called during object initialization.
    pub fn on_object_metadata_initialization<VM: VMBinding>(&self, object: ObjectReference) {
        // If it is in header, we have to set the mark bit for every newly allocated object
        if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_in_header() {
            VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
                object,
                self.unmarked_state(),
                None,
                Ordering::SeqCst,
            );
        }
    }

    /// This has to be called in the global preparation of a space
    pub fn on_global_prepare<VM: VMBinding>(&mut self) {}

    /// This has to be called when a space resets its memory regions. This can be either called before the GC tracing, or
    /// after a GC tracing (eagerly). This method will reset the mark bit. The policy should not use the mark bit before
    /// doing another tracing.
    pub fn on_block_reset<VM: VMBinding>(&self, start: Address, size: usize) {
        if let crate::util::metadata::MetadataSpec::OnSide(side) =
            *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
        {
            side.bzero_metadata(start, size);
        }
    }

    /// This has to be called in the global release of a space
    pub fn on_global_release<VM: VMBinding>(&mut self) {
        if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_in_header() {
            // If it is in header, we flip it. In this case, we do not need to reset the bits for marked objects
            self.state = self.unmarked_state()
        }
    }
}
