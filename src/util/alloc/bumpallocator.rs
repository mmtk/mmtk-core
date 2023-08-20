use super::BumpPointer;
use crate::util::Address;

use crate::util::alloc::Allocator;

use crate::plan::Plan;
use crate::policy::space::Space;
use crate::util::conversions::bytes_to_pages;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
pub struct BumpAllocator<VM: VMBinding> {
    /// [`VMThread`] associated with this allocator instance
    pub tls: VMThread,
    /// Bump-pointer itself.
    pub bump_pointer: BumpPointer<VM>,
    /// [`Space`](src/policy/space/Space) instance associated with this allocator instance.
    space: &'static dyn Space<VM>,
    /// [`Plan`] instance that this allocator instance is associated with.
    plan: &'static dyn Plan<VM = VM>,
}

impl<VM: VMBinding> BumpAllocator<VM> {
    pub fn set_limit(&mut self, start: Address, limit: Address) {
        self.bump_pointer.reset(start, limit);
    }

    pub fn reset(&mut self) {
        let zero = unsafe { Address::zero() };
        self.bump_pointer.reset(zero, zero);
    }

    pub fn rebind(&mut self, space: &'static dyn Space<VM>) {
        self.reset();
        self.space = space;
    }
}

impl<VM: VMBinding> Allocator<VM> for BumpAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        BLOCK_SIZE
    }

    fn alloc(&mut self, size: usize, align: usize, offset: usize) -> Address {
        trace!("alloc");
        let result = self.bump_pointer.alloc(size, align, offset);
        if result.is_zero() {
            trace!("Thread local buffer used up, go to alloc slow path");
            self.alloc_slow(size, align, offset)
        } else {
            trace!(
                "Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
                size,
                result,
                self.bump_pointer.cursor,
                self.bump_pointer.limit
            );
            result
        }
    }

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: usize) -> Address {
        trace!("alloc_slow");
        self.acquire_block(size, align, offset, false)
    }

    /// Slow path for allocation if precise stress testing has been enabled.
    /// It works by manipulating the limit to be always below the cursor.
    /// Can have three different cases:
    ///  - acquires a new block if the hard limit has been met;
    ///  - allocates an object using the bump pointer semantics from the
    ///    fastpath if there is sufficient space; and
    ///  - does not allocate an object but forces a poll for GC if the stress
    ///    factor has been crossed.
    fn alloc_slow_once_precise_stress(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        need_poll: bool,
    ) -> Address {
        if need_poll {
            return self.acquire_block(size, align, offset, true);
        }

        trace!("alloc_slow stress_test");
        // For stress test, limit is [0, block_size) to artificially make the
        // check in the fastpath (alloc()) fail. The real limit is recovered by
        // adding it to the current cursor.
        let result = self.bump_pointer.alloc(size, align, offset);

        if result.is_zero() {
            self.acquire_block(size, align, offset, true)
        } else {
            trace!(
                "alloc_slow: Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
                size,
                result,
                self.bump_pointer.cursor,
                self.bump_pointer.limit
            );
            result
        }
    }

    fn get_tls(&self) -> VMThread {
        self.tls
    }
}

impl<VM: VMBinding> BumpAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: &'static dyn Space<VM>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        BumpAllocator {
            tls,
            bump_pointer: unsafe { BumpPointer::new(Address::zero(), Address::zero()) },
            space,
            plan,
        }
    }

    fn acquire_block(
        &mut self,
        size: usize,
        align: usize,
        offset: usize,
        stress_test: bool,
    ) -> Address {
        if self.space.will_oom_on_acquire(self.tls, size) {
            return Address::ZERO;
        }

        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start = self.space.acquire(self.tls, bytes_to_pages(block_size));
        if acquired_start.is_zero() {
            trace!("Failed to acquire a new block");
            acquired_start
        } else {
            trace!(
                "Acquired a new block of size {} with start address {}",
                block_size,
                acquired_start
            );
            if !stress_test {
                self.set_limit(acquired_start, acquired_start + block_size);
                self.alloc(size, align, offset)
            } else {
                // For a stress test, we artificially make the fastpath fail by
                // manipulating the limit as below.
                // The assumption here is that we use an address range such that
                // cursor > block_size always.
                self.set_limit(acquired_start, unsafe { Address::from_usize(block_size) });
                // Note that we have just acquired a new block so we know that we don't have to go
                // through the entire allocation sequence again, we can directly call the slow path
                // allocation.
                self.alloc_slow_once_precise_stress(size, align, offset, false)
            }
        }
    }
}
