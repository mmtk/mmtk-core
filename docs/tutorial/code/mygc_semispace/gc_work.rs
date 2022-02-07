// ANCHOR: imports
use super::global::MyGC;
use crate::scheduler::gc_work::*;
use crate::vm::VMBinding;
use std::ops::{Deref, DerefMut};
// ANCHOR_END: imports

// ANCHOR: workcontext
pub struct MyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type ProcessEdgesWorkType = SFTProcessEdges<VM>;
}
// ANCHOR_END: workcontext
