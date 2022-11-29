use crate::plan::VectorQueue;
use crate::plan::barriers::BarrierSemantics;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;

use super::global::StickyImmix;

pub struct StickyImmixBarrierSemantics<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
    plan: &'static StickyImmix<VM>,
    modbuf: VectorQueue<ObjectReference>,
    region_modbuf: VectorQueue<ObjectReference>,
}

impl<VM: VMBinding> StickyImmixBarrierSemantics<VM> {
    pub fn new(mmtk: &'static MMTK<VM>, plan: &'static StickyImmix<VM>) -> Self {
        Self {
            mmtk,
            plan,
            modbuf: VectorQueue::new(),
            region_modbuf: VectorQueue::new(),
        }
    }
}

// impl<VM: VMBinding> BarrierSemantics for StickyImmixBarrierSemantics<VM> {
//     type VM = VM;


// }