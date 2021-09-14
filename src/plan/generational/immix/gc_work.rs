use super::global::GenImmix;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use crate::util::alloc::ImmixAllocator;
use std::ops::{Deref, DerefMut};

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
        self.defrag_copy.reset();
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
            copy: ImmixAllocator::new(VMThread::UNINITIALIZED, Some(&plan.immix), &*mmtk.plan, false),
            defrag_copy: ImmixAllocator::new(VMThread::UNINITIALIZED, Some(&plan.immix), &*mmtk.plan, true),
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for GenImmixCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}

// use crate::plan::immix::gc_work::TraceKind;
// pub struct GenImmixMatureProcessEdges<VM: VMBinding, const KIND: TraceKind> {
//     plan: &'static GenImmix<VM>,
//     base: ProcessEdgesBase<GenImmixMatureProcessEdges<VM, KIND>>,
// }

// impl<VM: VMBinding, const KIND: TraceKind> ProcessEdgesWork for GenImmixMatureProcessEdges<VM, KIND> {
//     type VM = VM;

//     fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
//         let base = ProcessEdgesBase::new(edges, mmtk);
//         let plan = base.plan().downcast_ref::<GenImmix<VM>>().unwrap();
//         Self { plan, base }
//     }


// }