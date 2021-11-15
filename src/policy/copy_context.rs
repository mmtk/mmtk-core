use crate::util::VMWorkerThread;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::plan::PlanConstraints;
use crate::plan::AllocationSemantics;
use crate::scheduler::GCWorkerLocal;
use std::marker::PhantomData;

/// A GC worker's context for copying GCs.
/// Each GC plan should provide their implementation of a CopyContext.
/// For non-copying GC, NoCopy can be used.
pub trait CopyContext: 'static + Send {
    type VM: VMBinding;
    fn constraints(&self) -> &'static PlanConstraints;
    fn init(&mut self, tls: VMWorkerThread);
    fn prepare(&mut self);
    fn release(&mut self);
    fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        semantics: AllocationSemantics,
    ) -> Address;
    fn post_copy(
        &mut self,
        _obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: AllocationSemantics,
    ) {
    }
    fn copy_check_allocator(
        &self,
        _from: ObjectReference,
        bytes: usize,
        align: usize,
        semantics: AllocationSemantics,
    ) -> AllocationSemantics {
        let large = crate::util::alloc::allocator::get_maximum_aligned_size::<Self::VM>(
            bytes,
            align,
            Self::VM::MIN_ALIGNMENT,
        ) > self.constraints().max_non_los_copy_bytes;
        if large {
            AllocationSemantics::Los
        } else {
            semantics
        }
    }
}

pub struct NoCopy<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> CopyContext for NoCopy<VM> {
    type VM = VM;

    fn init(&mut self, _tls: VMWorkerThread) {}
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
        _semantics: AllocationSemantics,
    ) -> Address {
        unreachable!()
    }
}

impl<VM: VMBinding> NoCopy<VM> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<VM: VMBinding> GCWorkerLocal for NoCopy<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}