use super::global::SemiSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::copy::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

use crate::scheduler::gc_work::MMTkProcessEdges;
pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type ProcessEdgesWorkType = MMTkProcessEdges<VM>;
}
