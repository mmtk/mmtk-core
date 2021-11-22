use crate::plan::AllocationSemantics;
use crate::plan::PlanConstraints;
use crate::scheduler::GCWorkerLocal;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use crate::util::copy::*;

/// A GC worker's copy allocator for copying GCs.
/// Each copying policy should provide their implementation of CopyContext.
/// For non-copying policy, they do not need a copy context. For them, NoCopy will be used.
/// If we copy objects from one policy to a different policy, the copy context of the destination
/// policy should be used. For example, for generational immix, the nursery is CopySpace, and the
/// mature space is ImmixSpace. When we copy from nursery to mature, ImmixCopyContext should be
/// used.
/// Currently we are assuming one plan will only use one copy context, which is true for our current GCs.
/// However, that might not be true in the future if we have more than 2 generations in a plan. We
/// may need to reconsider this design. Possibly we can do something similar to Mutator.
pub trait CopyContext: 'static + Send {
    type VM: VMBinding;
    fn constraints(&self) -> &'static PlanConstraints;
    fn prepare(&mut self);
    fn release(&mut self);
    fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        semantics: CopySemantics,
    ) -> Address;
    fn post_copy(
        &mut self,
        _obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        semantics: CopySemantics,
    ) {
    }
}

/// A stub implementation for CopyContext. This is used as per GC worker
/// thread local type for non copying GCs (which won't be used). It does nothing for most of its
/// methods, and will panic if alloc_copy() is ever called.
pub struct NoCopy<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> CopyContext for NoCopy<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        unreachable!()
    }
    fn prepare(&mut self) {}
    fn release(&mut self) {}
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        _bytes: usize,
        _align: usize,
        _offset: isize,
        _semantics: CopySemantics,
    ) -> Address {
        unreachable!()
    }
}

impl<VM: VMBinding> NoCopy<VM> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<VM: VMBinding> GCWorkerLocal for NoCopy<VM> {}

/// CopyDestination describes which policy we copy objects to.
/// A policy can use this to determine which copy context it should
/// use for copying.
#[derive(Copy, Clone, Debug)]
pub enum CopyDestination {
    CopySpace,
    ImmixSpace,
}
