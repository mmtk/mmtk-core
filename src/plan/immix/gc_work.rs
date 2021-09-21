use super::global::Immix;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, ImmixAllocator};
use crate::util::object_forwarding;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use crate::{
    plan::CopyContext,
    util::opaque_pointer::{VMThread, VMWorkerThread},
};
use std::ops::{Deref, DerefMut};

/// Immix copy allocator
pub struct ImmixCopyContext<VM: VMBinding> {
    immix: ImmixAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for ImmixCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::IMMIX_CONSTRAINTS
    }
    fn init(&mut self, tls: VMWorkerThread) {
        self.immix.tls = tls.0;
    }
    fn prepare(&mut self) {
        self.immix.reset()
    }
    fn release(&mut self) {
        self.immix.reset()
    }
    #[inline(always)]
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        _semantics: crate::AllocationSemantics,
    ) -> Address {
        self.immix.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(
        &mut self,
        obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: crate::AllocationSemantics,
    ) {
        object_forwarding::clear_forwarding_bits::<VM>(obj);
    }
}

impl<VM: VMBinding> ImmixCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            immix: ImmixAllocator::new(
                VMThread::UNINITIALIZED,
                Some(&mmtk.plan.downcast_ref::<Immix<VM>>().unwrap().immix_space),
                &*mmtk.plan,
                true,
            ),
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for ImmixCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub(in crate::plan) enum TraceKind {
    Fast,
    Defrag,
}

pub(super) struct ImmixProcessEdges<VM: VMBinding, const KIND: TraceKind> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static Immix<VM>,
    base: ProcessEdgesBase<Self>,
}

impl<VM: VMBinding, const KIND: TraceKind> ImmixProcessEdges<VM, KIND> {
    fn immix(&self) -> &'static Immix<VM> {
        self.plan
    }

    #[inline(always)]
    fn fast_trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.immix().immix_space.in_space(object) {
            self.immix().immix_space.fast_trace_object(self, object)
        } else {
            self.immix()
                .common
                .trace_object::<Self, ImmixCopyContext<VM>>(self, object)
        }
    }

    /// Trace objects without evacuation.
    #[inline(always)]
    fn fast_process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        self.fast_trace_object(object);
    }
}

impl<VM: VMBinding, const KIND: TraceKind> ProcessEdgesWork for ImmixProcessEdges<VM, KIND> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = crate::policy::immix::DEFRAG;

    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<Immix<VM>>().unwrap();
        Self { plan, base }
    }

    #[cold]
    fn flush(&mut self) {
        if self.nodes.is_empty() {
            return;
        }
        let scan_objects_work = crate::policy::immix::ScanObjectsAndMarkLines::<Self>::new(
            self.pop_nodes(),
            false,
            &self.immix().immix_space,
        );
        self.new_scan_work(scan_objects_work);
    }

    /// Trace  and evacuate objects.
    #[inline(always)]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.immix().immix_space.in_space(object) {
            self.immix().immix_space.trace_object(
                self,
                object,
                super::global::ALLOC_IMMIX,
                unsafe { self.worker().local::<ImmixCopyContext<VM>>() },
            )
        } else {
            self.immix()
                .common
                .trace_object::<Self, ImmixCopyContext<VM>>(self, object)
        }
    }

    #[inline]
    fn process_edges(&mut self) {
        if KIND == TraceKind::Fast {
            for i in 0..self.edges.len() {
                // Use fast_process_edge since we don't need to forward any objects.
                self.fast_process_edge(self.edges[i])
            }
        } else {
            for i in 0..self.edges.len() {
                self.process_edge(self.edges[i])
            }
        }
    }
}

impl<VM: VMBinding, const KIND: TraceKind> Deref for ImmixProcessEdges<VM, KIND> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, const KIND: TraceKind> DerefMut for ImmixProcessEdges<VM, KIND> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
