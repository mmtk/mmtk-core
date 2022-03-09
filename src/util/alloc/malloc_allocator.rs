use crate::policy::mallocspace::MallocSpace;
use crate::policy::space::Space;
use crate::util::alloc::object_ref_guard::object_ref_may_cross_chunk;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::VMBinding;
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

    fn does_thread_local_allocation(&self) -> bool {
        false
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        assert!(offset >= 0);

        let ret = self.space.alloc(self.tls, size, align, offset);
        if !object_ref_may_cross_chunk::<VM>(ret) {
            ret
        } else {
            // The address we got does not pass object ref checks. We cache it and free it later.
            // We free the results in the end to avoid malloc giving us the free'd address again.
            // The creation of the vec is put here so for the common case where we succeed in the first allocation,
            // we do not need to create this vec.
            let mut to_free = vec![ret];
            loop {
                let ret = self.space.alloc(self.tls, size, align, offset);
                if object_ref_may_cross_chunk::<VM>(ret) {
                    // The result does not pass check. Cache it.
                    to_free.push(ret);
                } else {
                    // The result passes the check. We free all the cached results, and return the new result.
                    for addr in to_free.iter() {
                        self.space.free(*addr);
                    }
                    return ret;
                }
            }
        }
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
