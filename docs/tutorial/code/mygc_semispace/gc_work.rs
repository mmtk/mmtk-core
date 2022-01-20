// ANCHOR: imports
use super::global::MyGC;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::copy::CopySemantics;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};
// ANCHOR_END: imports

// ANCHOR: mygc_process_edges
pub struct MyGCProcessEdges<VM: VMBinding> {
    plan: &'static MyGC<VM>,
    base: ProcessEdgesBase<VM>,
}
// ANCHOR_END: mygc_process_edges

impl<VM: VMBinding> MyGCProcessEdges<VM> {
    fn mygc(&self) -> &'static MyGC<VM> {
        self.plan
    }
}

impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM> {
    type VM = VM;
    // ANCHOR: mygc_process_edges_new
    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
        Self { base, plan }
    }
    // ANCHOR_END: mygc_process_edges_new

    // ANCHOR: trace_object
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.mygc().tospace().in_space(object) {
            self.mygc().tospace().trace_object::<Self>(
                self,
                object,
                CopySemantics::DefaultCopy,
                self.worker(),
            )
        } else if self.mygc().fromspace().in_space(object) {
            self.mygc().fromspace().trace_object::<Self>(
                self,
                object,
                CopySemantics::DefaultCopy,
                self.worker(),
            )
        } else {
            self.mygc().common.trace_object::<Self>(self, object)
        }
    }
    // ANCHOR_END: trace_object
}

// ANCHOR: deref
impl<VM: VMBinding> Deref for MyGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MyGCProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
// ANCHOR_END: deref

// ANCHOR: workcontext
pub struct MyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type ProcessEdgesWorkType = MyGCProcessEdges<VM>;
}
// ANCHOR_END: workcontext
