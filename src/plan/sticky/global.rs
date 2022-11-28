use crate::vm::VMBinding;
use crate::policy::space::Space;
use crate::plan::global::CommonPlan;

use std::marker::PhantomData;
use std::sync::atomic::AtomicBool;

pub struct Sticky<VM: VMBinding> {
    pub gc_full_heap: AtomicBool,
    pub next_gc_full_heap: AtomicBool,
    _p: PhantomData<VM>
}

impl<VM: VMBinding> Sticky<VM> {
    pub fn new() -> Self {
        Self {
            gc_full_heap: AtomicBool::new(false),
            next_gc_full_heap: AtomicBool::new(false),
            _p: PhantomData,
        }
    }
}
