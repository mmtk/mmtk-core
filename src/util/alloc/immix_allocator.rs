use crate::{policy::immix::ImmixSpace, util::constants::DEFAULT_STRESS_FACTOR};
use std::{ops::Add, sync::atomic::Ordering};

use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::Address;

use crate::util::alloc::Allocator;

use crate::plan::Plan;
use crate::policy::space::Space;
use crate::policy::immix::block::*;
use crate::policy::immix::line::*;
#[cfg(feature = "analysis")]
use crate::util::analysis::obj_size::PerSizeClassObjectCounterArgs;
#[cfg(feature = "analysis")]
use crate::util::analysis::RtAnalysis;
use crate::util::conversions::bytes_to_pages;
use crate::util::OpaquePointer;
use crate::vm::{ActivePlan, VMBinding};

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

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
    line_use_count: i32,
    mark_table: Address,
    recyclable_block: Address,
    line: i32,
    recyclable_exhausted: bool,
}

impl<VM: VMBinding> ImmixAllocator<VM> {
    pub fn reset(&mut self) {
        self.cursor = Address::ZERO;
        self.limit = Address::ZERO;
        self.large_cursor = Address::ZERO;
        self.large_limit = Address::ZERO;
        self.recyclable_block = Address::ZERO;
        self.request_for_large = false;
        self.recyclable_exhausted = false;
        self.line = Block::LINES as _;
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
        match self.space.unwrap().downcast_ref::<ImmixSpace<VM>>().unwrap().get_space(self.tls) {
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
            recyclable_block: Address::ZERO,
            line: 0,
            recyclable_exhausted: false,
        }
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
        false
    }
}
