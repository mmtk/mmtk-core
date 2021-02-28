use crate::{policy::immix::ImmixSpace, util::memory};
use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::Address;
use crate::util::alloc::Allocator;
use crate::plan::Plan;
use crate::policy::space::Space;
use crate::policy::immix::block::*;
use crate::policy::immix::line::*;
use crate::util::OpaquePointer;
use crate::vm::*;


#[repr(C)]
pub struct ImmixAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    /// bump pointer
    cursor: Address,
    /// limit for bump pointer
    limit: Address,
    space: Option<&'static dyn Space<VM>>,
    plan: &'static dyn Plan<VM = VM>,
    hot: bool,
    copy: bool,
    /// bump pointer for large objects
    large_cursor: Address,
    /// limit for bump pointer for large objects
    large_limit: Address,
    /// is the current request for large or small?
    request_for_large: bool,
    /// did the last allocation straddle a line?
    straddle: bool,
    /// approximation to bytes allocated
    line_use_count: usize,
    mark_table: Address,
    recyclable_block: Option<Block>,
    line: Option<Line>,
    recyclable_exhausted: bool,
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn reset(&mut self) {
        self.cursor = Address::ZERO;
        self.limit = Address::ZERO;
        self.large_cursor = Address::ZERO;
        self.large_limit = Address::ZERO;
        self.recyclable_block = None;
        self.request_for_large = false;
        self.recyclable_exhausted = false;
        self.line = None;
        self.line_use_count = 0;
    }
}

impl<VM: VMBinding> Allocator<VM> for ImmixAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
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
                self.overflow_alloc(size, align, offset)
            } else {
                self.alloc_slow_hot(size, align, offset)
            }
        } else {
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

    fn alloc_slow_once(&mut self, size: usize, align: usize, offset: isize) -> Address {
        match self.immix_space().get_clean_block(self.tls) {
            None => {
                self.line_use_count = 0;
                Address::ZERO
            },
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

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static dyn Plan<VM = VM>,
    ) -> Self {
        ImmixAllocator {
            tls,
            space,
            plan,
            cursor: Address::ZERO,
            limit: Address::ZERO,
            hot: false,
            copy: false,
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

    fn immix_space(&self) -> &'static ImmixSpace<VM> {
        self.space.unwrap().downcast_ref::<ImmixSpace<VM>>().unwrap()
    }

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

    #[cold]
    fn alloc_slow_hot(&mut self, size: usize, align: usize, offset: isize) -> Address {
        if self.acquire_recycable_lines(size, align, offset) {
            self.alloc(size, align, offset)
        } else {
            self.alloc_slow_inline(size, align, offset)
        }
    }

    fn acquire_recycable_lines(&mut self, size: usize, align: usize, offset: isize) -> bool {
        while self.line.is_some() || self.acquire_recycable_block() {
            if let Some(lines) = self.immix_space().get_next_available_lines(self.line.unwrap()) {
                self.cursor = lines.start.start();
                self.limit = lines.end.start();
                trace!("acquire_recycable_lines -> {:?} {:?} {:?}", self.line, lines, self.tls);
                memory::zero(self.cursor, self.limit - self.cursor);
                debug_assert!(align_allocation_no_fill::<VM>(self.cursor, align, offset) + size <= self.limit);
                self.line = if lines.end == self.recyclable_block.unwrap().lines().end { None } else { Some(lines.end) };
                return true;
            } else {
                self.line = None;
                self.recyclable_block = None;
            }
        }
        false
    }

    fn acquire_recycable_block(&mut self) -> bool {
        match self.immix_space().get_reusable_block() {
            Some(block) => {
                trace!("acquire_recycable_block -> {:?}", block);
                self.line = Some(block.lines().start);
                self.recyclable_block = Some(block);
                true
            }
            _ => false,
        }
    }
}
