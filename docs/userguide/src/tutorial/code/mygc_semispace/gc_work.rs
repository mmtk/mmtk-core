// ANCHOR: imports
use super::global::MyGC;
use crate::plan::tracing::{SFTTrace, Trace, UnsupportedTrace};
use crate::vm::VMBinding;
// ANCHOR_END: imports

// ANCHOR: workcontext_sft
pub struct MyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type DefaultTrace = SFTTrace<Self::VM>;
    type PinningTrace = UnsupportedTrace<Self::VM>;
}
// ANCHOR_END: workcontext_sft

// ANCHOR: workcontext_plan
use crate::plan::tracing::PlanTrace;
use crate::policy::gc_work::DEFAULT_TRACE;
pub struct MyGCWorkContext2<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext2<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type DefaultTrace = PlanTrace<MyGC<VM>, DEFAULT_TRACE>;
    type PinningTrace = UnsupportedTrace<Self::VM>;
}
// ANCHOR_END: workcontext_plan

use crate::policy::space::Space;
use crate::util::copy::CopySemantics;
use crate::util::ObjectReference;
use crate::MMTK;

// ANCHOR: mygc_trace
pub struct MyGCTrace<VM: VMBinding> {
    plan: &'static MyGC<VM>,
}
// ANCHOR_END: mygc_trace

// ANCHOR: mygc_trace_impl_clone
impl<VM: VMBinding> Clone for MyGCTrace<VM> {
    fn clone(&self) -> Self {
        Self { plan: self.plan }
    }
}
// ANCHOR_END: mygc_trace_impl_clone

// ANCHOR: mygc_trace_impl_trace
impl<VM: VMBinding> Trace for MyGCTrace<VM> {
    type VM = VM;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        // Instantiate `MyGCTrace` from a reference to `MMTK`.
        // We need to extract the plan reference, and it is sufficient to use downcast.
        Self {
            plan: mmtk.get_plan().downcast_ref().unwrap(),
        }
    }

    fn trace_object<Q: crate::ObjectQueue>(
        &self,
        worker: &mut crate::scheduler::GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        // We figure out which space the `object` is in,
        // and call the `trace_object` method of that space.
        if self.plan.tospace().in_space(object) {
            self.plan.tospace().trace_object(
                queue,
                object,
                Some(CopySemantics::DefaultCopy),
                worker,
            )
        } else if self.plan.fromspace().in_space(object) {
            self.plan.fromspace().trace_object(
                queue,
                object,
                Some(CopySemantics::DefaultCopy),
                worker,
            )
        } else {
            // If the `object` is in neither the fromspace nor the tospace,
            // we delegate to the `CommonPlan`.
            use crate::plan::PlanTraceObject;
            use crate::policy::gc_work::DEFAULT_TRACE;
            self.plan
                .common
                .trace_object::<_, DEFAULT_TRACE>(queue, object, worker)
        }
    }

    fn post_scan_object(&self, object: ObjectReference) {
        // Currently only `ImmixSpace` needs `post_scan_object`.
        // `CopySpace` does not need the `post_scan_object` method,
        // so we don't need to call `post_scan_object` on the fromspace or the tospace.

        // We need to call the `post_scan_object` method of the common plan
        // because by default the non-moving space is an `ImmixSpace`.
        use crate::plan::PlanTraceObject;
        self.plan.common.post_scan_object(object);
    }

    fn may_move_objects() -> bool {
        // We return `true` because SemiSpace moves every single reachable object in the from space.
        true
    }
}
// ANCHOR_END: mygc_trace_impl_trace

// ANCHOR: workcontext_mygc
pub struct MyGCWorkContext3<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext3<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type DefaultTrace = MyGCTrace<Self::VM>;
    type PinningTrace = UnsupportedTrace<Self::VM>;
}
// ANCHOR: workcontext_mygc
