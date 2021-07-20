use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::plan::Plan;
use crate::policy::immix::block::*;
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
    /// Did the last allocation straddle a line?
    ///
    /// *unused*
    straddle: bool,
    /// approximation to bytes allocated
    line_use_count: usize,
    /// Line mark table
    mark_table: Address,
    /// Current recyclable block for allocation
    recyclable_block: Option<Block>,
    /// Hole-searching cursor
    line: Option<Line>,
    recyclable_exhausted: bool,
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn reset(&mut self) {
        self.cursor = Address::ZERO;
        self.limit = Address::ZERO;
        self.large_cursor = Address::ZERO;
        self.large_limit = Address::ZERO;
        self.mark_table = Address::ZERO;
        self.recyclable_block = None;
        self.request_for_large = false;
        self.recyclable_exhausted = false;
        self.line = None;
        self.line_use_count = 0;
    }
}

impl<VM: VMBinding> Allocator<VM> for ImmixAllocator<VM> {
    fn get_space(&self) -> &'static dyn Space<VM> {
        self.space as _
    }

    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    #[inline(always)]
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc");
        let result = align_allocation_no_fill::<VM>(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            trace!("Thread local buffer used up, go to alloc slow path");
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
                "Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
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
        match self.immix_space().get_clean_block(self.tls, self.copy) {
            None => {
                self.line_use_count = 0;
                Address::ZERO
            }
            Some(block) => {
                self.line_use_count = Block::LINES as _;
                trace!("Acquired a new block {:?}", block);
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
            straddle: false,
            line_use_count: 0,
            mark_table: Address::ZERO,
            recyclable_block: None,
            line: None,
            recyclable_exhausted: false,
        }
    }

    #[inline(always)]
    fn immix_space(&self) -> &'static ImmixSpace<VM> {
        self.space
    }

    /// Large-object (larger than a line) bump alloaction.
    fn overflow_alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
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
                    "acquire_recyclable_lines -> {:?} {:?} {:?}",
                    self.line,
                    lines,
                    self.tls
                );
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
                trace!("acquire_recyclable_block -> {:?}", block);
                // Set the hole-searching cursor to the start of this block.
                self.line = Some(block.lines().start);
                true
            }
            _ => false,
        }
    }
}
