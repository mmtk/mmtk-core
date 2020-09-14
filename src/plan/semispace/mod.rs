mod collector;
pub mod constraints;
mod global;
mod mutator;
mod tracelocal;

pub use self::collector::SSCollector;
pub use self::global::SemiSpace;
pub use self::mutator::SSMutator;
pub use self::tracelocal::SSTraceLocal;

pub use self::collector::SSCollector as SelectedCollector;
pub use self::constraints as SelectedConstraints;
pub use self::global::SelectedPlan;
pub use self::mutator::SSMutator as SelectedMutator;
pub use self::tracelocal::SSTraceLocal as SelectedTraceLocal;


use crate::work::*;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use crate::policy::space::Space;
use crate::util::alloc::{BumpAllocator, Allocator};
use crate::util::forwarding_word;
use crate::MMTK;
use crate::plan::CopyContext;

pub struct SSCopyContext<VM: VMBinding> {
    plan: &'static SemiSpace<VM>,
    ss: BumpAllocator<VM>,
}

impl <VM: VMBinding> CopyContext for SSCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: &mmtk.plan,
            ss: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn prepare(&mut self) {
        self.ss.rebind(Some(self.plan.tospace()));
    }
    fn release(&mut self) {
        // self.ss.rebind(Some(self.plan.tospace()));
    }
    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: crate::Allocator) -> Address {
        self.ss.alloc(bytes, align, offset)
    }
    fn post_copy(&mut self, obj: ObjectReference, _tib: Address, _bytes: usize, _allocator: crate::Allocator) {
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}

#[derive(Default)]
struct SSProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<SSProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> ProcessEdgesWork for SSProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_object(self, object, global::ALLOC_SS, self.worker().context());
        }
        if self.plan().fromspace().in_space(object) {
            return self.plan().fromspace().trace_object(self, object, global::ALLOC_SS, self.worker().context());
        }
        self.plan().common.trace_object(self, object)
    }
}

impl <VM: VMBinding> Deref for SSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for SSProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}