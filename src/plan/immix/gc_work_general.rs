use super::global::Immix;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::copy::CopySemantics;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub(in crate::plan) enum TraceKind {
    Fast,
    Defrag,
}

use crate::scheduler::gc_work::MMTkProcessEdges;
pub(super) struct ImmixGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ImmixGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = Immix<VM>;
    type ProcessEdgesWorkType = MMTkProcessEdges<VM>;
}
