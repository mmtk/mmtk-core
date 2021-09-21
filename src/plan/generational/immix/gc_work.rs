use super::global::GenImmix;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::Allocator;
use crate::util::alloc::ImmixAllocator;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::AllocationSemantics;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

/// Copy context for generational immix. We include two copy allocators for the same immix space.
/// We should use the defrag copy allocator for full heap GC with defrag, or the normal copy allocator
/// for other GCs.
pub struct GenImmixCopyContext<VM: VMBinding> {
    plan: &'static GenImmix<VM>,
    copy: ImmixAllocator<VM>,
    defrag_copy: ImmixAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for GenImmixCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::GENIMMIX_CONSTRAINTS
    }

    fn init(&mut self, tls: VMWorkerThread) {
        self.copy.tls = tls.0;
        self.defrag_copy.tls = tls.0;
    }

    fn prepare(&mut self) {
        self.copy.reset();
        if !self.plan.gen.is_current_gc_nursery() {
            self.defrag_copy.reset();
        }
    }

    fn release(&mut self) {
        self.copy.reset();
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
        debug_assert!(
            bytes <= super::GENIMMIX_CONSTRAINTS.max_non_los_default_alloc_bytes,
            "Attempted to copy an object of {} bytes (> {}) which should be allocated with LOS and not be copied.",
            bytes, super::GENIMMIX_CONSTRAINTS.max_non_los_default_alloc_bytes,
        );
        debug_assert!(VM::VMActivePlan::global().base().gc_in_progress_proper());
        if self.plan.immix.in_defrag() {
            self.defrag_copy.alloc(bytes, align, offset)
        } else {
            self.copy.alloc(bytes, align, offset)
        }
    }

    #[inline(always)]
    fn post_copy(
        &mut self,
        obj: ObjectReference,
        tib: Address,
        bytes: usize,
        semantics: crate::AllocationSemantics,
    ) {
        // Missing ImmixSpace.post_copy()
        crate::plan::generational::generational_post_copy::<VM>(obj, tib, bytes, semantics)
    }
}

impl<VM: VMBinding> GenImmixCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        let plan = &mmtk.plan.downcast_ref::<GenImmix<VM>>().unwrap();
        Self {
            plan,
            // it doesn't matter which space we bind with the copy allocator. We will rebind to a proper space in prepare().
            copy: ImmixAllocator::new(
                VMThread::UNINITIALIZED,
                Some(&plan.immix),
                &*mmtk.plan,
                false,
            ),
            defrag_copy: ImmixAllocator::new(
                VMThread::UNINITIALIZED,
                Some(&plan.immix),
                &*mmtk.plan,
                true,
            ),
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for GenImmixCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}

use crate::plan::immix::gc_work::TraceKind;

/// ProcessEdges for a full heap GC for generational immix. The const type parameter
/// defines whether there is copying in the GC.
/// Note that even with TraceKind::Fast, there is no defragmentation, we are still
/// copying from nursery to immix space. So we always need to write new object
/// references in process_edge() (i.e. we do not need to overwrite the default implementation
/// of process_edge() as the immix plan does).
pub(super) struct GenImmixMatureProcessEdges<VM: VMBinding, const KIND: TraceKind> {
    plan: &'static GenImmix<VM>,
    base: ProcessEdgesBase<GenImmixMatureProcessEdges<VM, KIND>>,
}

impl<VM: VMBinding, const KIND: TraceKind> ProcessEdgesWork
    for GenImmixMatureProcessEdges<VM, KIND>
{
    type VM = VM;

    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<GenImmix<VM>>().unwrap();
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
            &self.plan.immix,
        );
        self.new_scan_work(scan_objects_work);
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }

        if self.plan.immix.in_space(object) {
            if KIND == TraceKind::Fast {
                return self.plan.immix.fast_trace_object(self, object);
            } else {
                return self.plan.immix.trace_object(
                    self,
                    object,
                    AllocationSemantics::Default,
                    unsafe { self.worker().local::<GenImmixCopyContext<VM>>() },
                );
            }
        }

        self.plan
            .gen
            .trace_object_full_heap::<Self, GenImmixCopyContext<VM>>(self, object, unsafe {
                self.worker().local::<GenImmixCopyContext<VM>>()
            })
    }
}

impl<VM: VMBinding, const KIND: TraceKind> Deref for GenImmixMatureProcessEdges<VM, KIND> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, const KIND: TraceKind> DerefMut for GenImmixMatureProcessEdges<VM, KIND> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
