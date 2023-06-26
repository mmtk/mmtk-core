use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;

/// A GC worker's copy allocator for copying GCs.
/// Each copying policy should provide their implementation of PolicyCopyContext.
/// If we copy objects from one policy to a different policy, the copy context of the destination
/// policy should be used. For example, for generational immix, the nursery is CopySpace, and the
/// mature space is ImmixSpace. When we copy from nursery to mature, ImmixCopyContext should be
/// used.
/// Note that this trait should only be implemented with policy specific behaviors. Please
/// refer to [`crate::util::copy::GCWorkerCopyContext`] which implements common
/// behaviors for copying.
pub trait PolicyCopyContext: 'static + Send {
    type VM: VMBinding;
    fn prepare(&mut self);
    fn release(&mut self);
    fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: usize,
    ) -> Address;
    fn post_copy(&mut self, _obj: ObjectReference, _bytes: usize) {}
}
