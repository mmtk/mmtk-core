use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::plan::Plan;
use crate::policy::immix::line::*;
use crate::policy::immix::ImmixSpace;
use crate::policy::space::Space;
use crate::util::alloc::Allocator;
use crate::util::opaque_pointer::VMThread;
use crate::util::Address;
use crate::vm::*;

/// Immix allocator
#[repr(C)]
pub struct ImmixAllocator<VM: VMBinding> {
    pub tls: VMThread,
    /// Bump pointer
    cursor: Address,
    /// Limit for bump pointer
    limit: Address,
    space: &'static ImmixSpace<VM>,
    plan: &'static dyn Plan<VM = VM>,
    /// *unused*
    hot: bool,
    /// Is this a copy allocator?
    copy: bool,
    /// Bump pointer for large objects
    large_cursor: Address,
    /// Limit for bump pointer for large objects
    large_limit: Address,
    /// Is the current request for large or small?
    request_for_large: bool,
    /// Hole-searching cursor
    line: Option<Line>,
    /// Are we doing alloc slow when stress test is turned on. This is only set to true,
    /// during the allow_slow_once_stress_test() call. In the call, we will restore the correct
    /// limit for bump allocation, and call alloc() to try resolve the allocation request with
    /// the thread local buffer. If we cannot do the allocation from the thread local buffer,
    /// we will eventually call allow_slow_once_stress_test(). With this flag set to true, we know
    /// we are resolving an allocation request and have failed the thread local allocation. In
    /// this case, we will acquire new block from the space.
    alloc_slow_for_stress: bool,
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn reset(&mut self) {
        self.cursor = Address::ZERO;
        self.limit = Address::ZERO;
        self.large_cursor = Address::ZERO;
        self.large_limit = Address::ZERO;
        self.request_for_large = false;
        self.line = None;
    }
}

impl<VM: VMBinding> Allocator<VM> for ImmixAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space as _
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn does_thread_local_allocation(&self) -> bool {
        true
    }

    fn get_thread_local_buffer_granularity(&self) -> usize {
        crate::policy::immix::block::Block::BYTES
    }

    #[inline(always)]
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        debug_assert!(
            size <= crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
            "Trying to allocate a {} bytes object, which is larger than MAX_IMMIX_OBJECT_SIZE {}",
            size,
            crate::policy::immix::MAX_IMMIX_OBJECT_SIZE
        );
        let result = align_allocation_no_fill::<VM>(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            trace!(
                "{:?}: Thread local buffer used up, go to alloc slow path",
                self.tls
            );
            if size > Line::BYTES {
                // Size larger than a line: do large allocation
                self.overflow_alloc(size, align, offset)
            } else {
                // Size smaller than a line: fit into holes
                self.alloc_slow_hot(size, align, offset)
            }
        } else {
            // Simple bump allocation.
            fill_alignment_gap::<VM>(self.cursor, result);
            self.cursor = new_cursor;
            trace!(
                "{:?}: Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
                self.tls,
                size,
                result,
                self.cursor,
                self.limit
            );
            result
        }
    }

    /// Acquire a clean block from ImmixSpace for allocation.
    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("{:?}: alloc_slow_once", self.tls);
        self.acquire_clean_block(size, align, offset)
    }

    fn alloc_slow_once_stress_test(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        need_poll: bool,
    ) -> Address {
        trace!("{:?}: alloc_slow_once_stress_test", self.tls);
        // If we are required to make a poll, we call acquire_clean_block() which will acquire memory
        // from the space which includes a GC poll.
        if need_poll {
            trace!("{:?}: alloc_slow_once_stress_test going to poll", self.tls);
            let ret = self.acquire_clean_block(size, align, offset);
            // Set fake limits so later allocation will fail in the fastpath, and end up going to this
            // special slowpath.
            self.set_limit_for_stress();
            trace!(
                "{:?}: alloc_slow_once_stress_test done - forced stress poll",
                self.tls
            );
            return ret;
        }

        // We are not yet required to do a stress GC. We will try to allocate from thread local buffer if possible.
        // Restore the fake limit to the normal limit so we can do thread local alloaction normally.
        self.restore_limit_for_stress();
        let ret = if self.alloc_slow_for_stress {
            // If we are already doing allow_slow for stress test, and reach here, it means we have failed the
            // thread local allocation, and we have to get a new block from the space.
            trace!(
                "{:?}: alloc_slow_once_stress_test - acquire new block",
                self.tls
            );
            self.acquire_clean_block(size, align, offset)
        } else {
            // Indicate that we are doing alloc slow for stress test. If the alloc() cannot allocate from
            // thread local buffer, we will reach this method again. In that case, we will need to poll, rather
            // than attempting to alloc() again.
            self.alloc_slow_for_stress = true;
            // Try allocate. The allocator will try allocate from thread local buffer, if that fails, it will
            // get a clean block.
            trace!("{:?}: alloc_slow_once_stress_test - alloc()", self.tls);
            let ret = self.alloc(size, align, offset);
            // Indicate that we finish the alloc slow for stress test.
            self.alloc_slow_for_stress = false;
            ret
        };
        // Set fake limits
        self.set_limit_for_stress();
        ret
    }

    fn get_tls(&self) -> VMThread {
        self.tls
    }
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn new(
        tls: VMThread,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static dyn Plan<VM = VM>,
        copy: bool,
    ) -> Self {
        ImmixAllocator {
            tls,
            space: space.unwrap().downcast_ref::<ImmixSpace<VM>>().unwrap(),
            plan,
            cursor: Address::ZERO,
            limit: Address::ZERO,
            hot: false,
            copy,
            large_cursor: Address::ZERO,
            large_limit: Address::ZERO,
            request_for_large: false,
            line: None,
            alloc_slow_for_stress: false,
        }
    }

    #[inline(always)]
    fn immix_space(&self) -> &'static ImmixSpace<VM> {
        self.space
    }

    /// Large-object (larger than a line) bump alloaction.
    fn overflow_alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("{:?}: overflow_alloc", self.tls);
        let start = align_allocation_no_fill::<VM>(self.large_cursor, align, offset);
        let end = start + size;
        if end > self.large_limit {
            self.request_for_large = true;
            let rtn = self.alloc_slow_inline(size, align, offset);
            self.request_for_large = false;
            rtn
        } else {
            fill_alignment_gap::<VM>(self.large_cursor, start);
            self.large_cursor = end;
            start
        }
    }

    /// Bump allocate small objects into recyclable lines (i.e. holes).
    #[cold]
    fn alloc_slow_hot(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("{:?}: alloc_slow_hot", self.tls);
        if self.acquire_recyclable_lines(size, align, offset) {
            self.alloc(size, align, offset)
        } else {
            self.alloc_slow_inline(size, align, offset)
        }
    }

    /// Search for recyclable lines.
    fn acquire_recyclable_lines(&mut self, size: usize, align: usize, offset: isize) -> bool {
        while self.line.is_some() || self.acquire_recyclable_block() {
            let line = self.line.unwrap();
            if let Some(lines) = self.immix_space().get_next_available_lines(line) {
                // Find recyclable lines. Update the bump allocation cursor and limit.
                self.cursor = lines.start.start();
                self.limit = lines.end.start();
                trace!(
                    "{:?}: acquire_recyclable_lines -> {:?} {:?} {:?}",
                    self.tls,
                    self.line,
                    lines,
                    self.tls
                );
                #[cfg(feature = "global_alloc_bit")]
                crate::util::alloc_bit::bzero_alloc_bit(self.cursor, self.limit - self.cursor);
                crate::util::memory::zero(self.cursor, self.limit - self.cursor);
                debug_assert!(
                    align_allocation_no_fill::<VM>(self.cursor, align, offset) + size <= self.limit
                );
                let block = line.block();
                self.line = if lines.end == block.lines().end {
                    // Hole searching reached the end of a reusable block. Set the hole-searching cursor to None.
                    None
                } else {
                    // Update the hole-searching cursor to None.
                    Some(lines.end)
                };
                return true;
            } else {
                // No more recyclable lines. Set the hole-searching cursor to None.
                self.line = None;
            }
        }
        false
    }

    /// Get a recyclable block from ImmixSpace.
    fn acquire_recyclable_block(&mut self) -> bool {
        match self.immix_space().get_reusable_block(self.copy) {
            Some(block) => {
                trace!("{:?}: acquire_recyclable_block -> {:?}", self.tls, block);
                // Set the hole-searching cursor to the start of this block.
                self.line = Some(block.lines().start);
                true
            }
            _ => false,
        }
    }

    // Get a clean block from ImmixSpace.
    fn acquire_clean_block(&mut self, size: usize, align: usize, offset: isize) -> Address {
        match self.immix_space().get_clean_block(self.tls, self.copy) {
            None => Address::ZERO,
            Some(block) => {
                trace!("{:?}: Acquired a new block {:?}", self.tls, block);
                if self.request_for_large {
                    self.large_cursor = block.start();
                    self.large_limit = block.end();
                } else {
                    self.cursor = block.start();
                    self.limit = block.end();
                }
                self.alloc(size, align, offset)
            }
        }
    }

    /// Set fake limits for the bump allocation for stress tests. The fake limit is the remaining thread local buffer size,
    /// which should be always smaller than the bump cursor.
    /// This method may be reentrant. We need to check before setting the values.
    fn set_limit_for_stress(&mut self) {
        if self.cursor < self.limit {
            let new_limit = unsafe { Address::from_usize(self.limit - self.cursor) };
            self.limit = new_limit;
            trace!(
                "{:?}: set_limit_for_stress. normal {} -> {}",
                self.tls,
                self.limit,
                new_limit
            );
        }
        if self.large_cursor < self.large_limit {
            let new_lg_limit = unsafe { Address::from_usize(self.large_limit - self.large_cursor) };
            self.large_limit = new_lg_limit;
            trace!(
                "{:?}: set_limit_for_stress. large {} -> {}",
                self.tls,
                self.large_limit,
                new_lg_limit
            );
        }
    }

    /// Restore the real limits for the bump allocation so we can do a properly thread local allocation.
    /// The fake limit is the remaining thread local buffer size, and we restore the actual limit from the size and the cursor.
    /// This method may be reentrant. We need to check before setting the values.
    fn restore_limit_for_stress(&mut self) {
        if self.limit < self.cursor {
            let new_limit = self.cursor + self.limit.as_usize();
            self.limit = new_limit;
            trace!(
                "{:?}: restore_limit_for_stress. normal {} -> {}",
                self.tls,
                self.limit,
                new_limit
            );
        }
        if self.large_limit < self.large_cursor {
            let new_lg_limit = self.large_cursor + self.large_limit.as_usize();
            self.large_limit = new_lg_limit;
            trace!(
                "{:?}: restore_limit_for_stress. large {} -> {}",
                self.tls,
                self.large_limit,
                new_lg_limit
            );
        }
    }
}
