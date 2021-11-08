use super::BumpAllocator;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::*;
use crate::util::Address;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;

// A thin wrapper of bump allocator
// reserve extra bytes when allocating
#[repr(C)]
pub struct MarkCompactAllocator<VM: VMBinding> {
    bump_allocator: BumpAllocator<VM>,
}

impl<VM: VMBinding> MarkCompactAllocator<VM> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.bump_allocator.set_limit(cursor, limit);
    }

    pub fn reset(&mut self) {
        self.bump_allocator.reset();
    }

    pub fn rebind(&mut self, space: &'static dyn Space<VM>) {
        self.bump_allocator.rebind(space);
    }
}

impl<VM: VMBinding> Allocator<VM> for MarkCompactAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.bump_allocator.get_space()
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.bump_allocator.get_plan()
    }

    fn get_tls(&self) -> VMThread {
        self.bump_allocator.get_tls()
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        self.bump_allocator.get_thread_local_buffer_granularity()
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let gc_extra_header_words = self.get_plan().constraints().gc_extra_header_words;
        let extra_header = if gc_extra_header_words != 0 {
            std::cmp::max(
                gc_extra_header_words * crate::util::constants::BYTES_IN_WORD,
                VM::VMObjectModel::object_alignment() as usize,
            )
            .next_power_of_two()
        } else {
            0
        };

        let rtn = self
            .bump_allocator
            .alloc(size + extra_header, align, offset);
        // return the actual object start address
        rtn + extra_header
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc_slow");
        self.bump_allocator.alloc_slow_once(size, align, offset)
    }

    // Slow path for allocation if the precise stress test has been enabled.
    // It works by manipulating the limit to be below the cursor always.
    // Performs three kinds of allocations: (i) if the hard limit has been met;
    // (ii) the bump pointer semantics from the fastpath; and (iii) if the stress
    // factor has been crossed.
    fn alloc_slow_once_precise_stress(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        need_poll: bool,
    ) -> Address {
        self.bump_allocator
            .alloc_slow_once_precise_stress(size, align, offset, need_poll)
    }
}

impl<VM: VMBinding> MarkCompactAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static dyn Space<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        MarkCompactAllocator {
            bump_allocator: BumpAllocator::new(tls, space, plan),
        }
    }
}
