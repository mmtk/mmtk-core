use crate::util::constants::DEFAULT_STRESS_FACTOR;
use std::sync::atomic::Ordering;

use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::Address;

use crate::util::alloc::Allocator;

use crate::plan::global::Plan;
use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::conversions::bytes_to_pages;
use crate::util::OpaquePointer;
use crate::vm::{ActivePlan, VMBinding};

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
pub struct BumpAllocator<VM: VMBinding> {
    pub tls: OpaquePointer,
    cursor: Address,
    limit: Address,
    space: Option<&'static dyn Space<VM>>,
    plan: &'static SelectedPlan<VM>,
}

impl<VM: VMBinding> BumpAllocator<VM> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    pub fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }

    pub fn rebind(&mut self, space: Option<&'static dyn Space<VM>>) {
        self.reset();
        self.space = space;
    }
}

impl<VM: VMBinding> Allocator<VM> for BumpAllocator<VM> {
    fn get_space(&self) -> Option<&'static dyn Space<VM>> {
        self.space
    }
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
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
        trace!("alloc_slow");
        // TODO: internalLimit etc.
        let base = &self.plan.base();

        if base.options.stress_factor == DEFAULT_STRESS_FACTOR {
            self.acquire_block(size, align, offset, false)
        } else {
            self.alloc_slow_once_stress_test(size, align, offset)
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
}

impl<VM: VMBinding> BumpAllocator<VM> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static dyn Space<VM>>,
        plan: &'static SelectedPlan<VM>,
    ) -> Self {
        BumpAllocator {
            tls,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
            plan,
        }
    }

    fn alloc_slow_once_stress_test(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc_slow stress_test");
        let result = align_allocation_no_fill::<VM>(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.cursor + self.limit.as_usize() {
            self.acquire_block(size, align, offset, true)
        } else {
            let base = &self.plan.base();
            let is_mutator =
                unsafe { VM::VMActivePlan::is_mutator(self.tls) } && self.plan.is_initialized();

            if is_mutator
                && (base.allocation_count.load(Ordering::Relaxed) > base.options.stress_factor)
            {
                trace!(
                    "Stress GC: allocation_count = {} more than stress_factor = {}",
                    base.allocation_count.load(Ordering::Relaxed),
                    base.options.stress_factor
                );
                return self.acquire_block(size, align, offset, true);
            }

            fill_alignment_gap::<VM>(self.cursor, result);
            let alloc_size = new_cursor - self.cursor;

            if is_mutator {
                let current_allocation_count = base.allocation_count.load(Ordering::Relaxed);
                base.allocation_count
                    .store(current_allocation_count + alloc_size, Ordering::Relaxed);
                trace!(
                    "Stress GC: allocation_count = {}",
                    base.allocation_count.load(Ordering::Relaxed)
                );
            }

            self.limit -= alloc_size;
            self.cursor = new_cursor;
            trace!(
                "alloc_slow: Bump allocation size: {}, result: {}, new_cursor: {}, limit: {}",
                size,
                result,
                self.cursor,
                self.limit
            );
            result
        }
    }

    #[inline]
    fn acquire_block(
        &mut self,
        size: usize,
        align: usize,
        offset: isize,
        stress_test: bool,
    ) -> Address {
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start = self
            .space
            .unwrap()
            .acquire(self.tls, bytes_to_pages(block_size));
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
            } else {
                self.set_limit(acquired_start, unsafe { Address::from_usize(block_size) });
            }
            self.alloc(size, align, offset)
        }
    }
}
