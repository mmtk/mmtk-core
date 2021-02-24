use crate::{policy::immix::ImmixSpace, util::constants::DEFAULT_STRESS_FACTOR};
use std::sync::atomic::Ordering;

use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::Address;

use crate::util::alloc::Allocator;

use crate::plan::Plan;
use crate::policy::space::Space;
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
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    pub fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }
}

impl<VM: VMBinding> Allocator<VM> for ImmixAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
    }
    fn get_plan(&self) -> &'static dyn Plan<VM = VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc");
        let result = align_allocation_no_fill::<VM>(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            trace!("Thread local buffer used up, go to alloc slow path");
            self.alloc_slow(size, align, offset)
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
        println!("alloc_slow start");
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start = self.space.unwrap().downcast_ref::<ImmixSpace<VM>>().unwrap().get_space(self.tls);
        println!("alloc_slow end");
        if acquired_start.is_zero() {
            println!("Failed to acquire a new block");
            acquired_start
        } else {
            trace!(
                "Acquired a new block of size {} with start address {}",
                block_size,
                acquired_start
            );
            self.set_limit(acquired_start, acquired_start + (8usize * 4096));
            self.alloc(size, align, offset)
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
}
