use super::global::Immix;
use crate::policy::immix::gc_work::{ImmixProcessEdgesWork, TraceKind};
use crate::vm::VMBinding;

pub(super) struct ImmixGCWorkContext<VM: VMBinding, const KIND: TraceKind>(
    std::marker::PhantomData<VM>,
);
impl<VM: VMBinding, const KIND: TraceKind> crate::scheduler::GCWorkContext
    for ImmixGCWorkContext<VM, KIND>
{
    type VM = VM;
    type PlanType = Immix<VM>;
    type ProcessEdgesWorkType = ImmixProcessEdgesWork<VM, Immix<VM>, KIND>;
}
