use crate::{MMTK, plan::marksweep::metadata::is_marked};
use crate::plan::marksweep::metadata::set_mark_bit;
use crate::scheduler::gc_works::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use std::ops::{Deref, DerefMut};

use super::MarkSweep;

pub struct MSProcessEdges<VM: VMBinding> {
    plan: &'static MarkSweep<VM>,
    base: ProcessEdgesBase<MSProcessEdges<VM>>,
}

impl<VM: VMBinding> ProcessEdgesWork for MSProcessEdges<VM> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<MarkSweep<VM>>().unwrap();
        Self {
            plan,
            base,
        }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if !is_marked(object) {
            set_mark_bit(object.to_address());
            self.process_node(object);
        }
        object
    }
}

impl<VM: VMBinding> Deref for MSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MSProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
