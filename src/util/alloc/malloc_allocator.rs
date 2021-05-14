#[cfg(feature = "analysis")]
use std::sync::atomic::Ordering;
use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;
#[cfg(feature = "analysis")]
use crate::vm::ActivePlan;
use crate::Plan;

#[repr(C)]
pub struct MallocAllocator<VM: VMBinding> {
    pub tls: VMThread,
    space: &'static MallocSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> Allocator<VM> for MallocAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space as &'static dyn Space<VM>
    }
    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.alloc_slow(size, align, offset)
    }

    fn get_tls(&self) -> VMThread {
        self.tls
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        // TODO: We currently ignore the offset field. This is wrong.
        // assert!(offset == 0);
        assert!(align <= 16);

        #[cfg(feature = "analysis")]
        {
            let base = &self.plan.base();
            let is_mutator =
                unsafe { VM::VMActivePlan::is_mutator(self.tls) } && self.plan.is_initialized();

            if is_mutator
                && base.allocation_bytes.load(Ordering::SeqCst) > base.options.analysis_factor
            {
                trace!(
                    "Analysis: allocation_bytes = {} more than analysis_factor = {}",
                    base.allocation_bytes.load(Ordering::Relaxed),
                    base.options.analysis_factor
                );

                base.analysis_manager.alloc_hook(size, align, offset);
            }
        }

        let ret = self.space.unwrap().alloc(self.tls, size);

        trace!(
            "MallocSpace.alloc size = {}, align = {}, offset = {}, res = {}",
            size,
            align,
            offset,
            ret
        );
        ret
    }
}

impl<VM: VMBinding> MallocAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static MallocSpace<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MallocAllocator { tls, space, plan }
    }
}
