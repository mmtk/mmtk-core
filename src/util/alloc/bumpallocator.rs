use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use crate::util::{Address, ObjectReference};

use crate::util::alloc::dump_linear_scan::DumpLinearScan;
use crate::util::alloc::linear_scan::LinearScan;
use crate::util::alloc::Allocator;
use crate::util::heap::PageResource;

use crate::vm::ObjectModel;

use crate::plan::selected_plan::SelectedPlan;
use crate::policy::space::Space;
use crate::util::conversions::bytes_to_pages;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

#[repr(C)]
pub struct BumpAllocator<VM: VMBinding, PR: PageResource<VM>> {
    pub tls: OpaquePointer,
    cursor: Address,
    limit: Address,
    space: Option<&'static PR::Space>,
    plan: &'static SelectedPlan<VM>,
}

impl<VM: VMBinding, PR: PageResource<VM>> BumpAllocator<VM, PR> {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    fn reset(&mut self) {
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }

    pub fn rebind(&mut self, space: Option<&'static PR::Space>) {
        self.reset();
        self.space = space;
    }

    pub fn scan(&self) {
        unsafe {
            self.scan_region(
                DumpLinearScan {},
                self.get_space().unwrap().common().start,
                Address::zero(),
            );
        }
    }

    fn scan_region<T: LinearScan>(&self, scanner: T, start: Address, end: Address) {
        // We are diverging from the original implementation
        let current_limit = if end.is_zero() { self.cursor } else { end };

        let mut current: ObjectReference =
            unsafe { VM::VMObjectModel::get_object_from_start_address(start) };

        println!("start: {}, first object: {}", start, current);

        /* Loop through each object up to the limit */
        loop {
            /* Read end address first, as scan may be destructive */
            let current_object_end: Address = VM::VMObjectModel::get_object_end_address(current);
            println!("current object: {} end: {}", current, current_object_end);
            scanner.scan::<VM>(current);
            if current_object_end > current_limit {
                /* We have scanned the last object */
                break;
            }

            /* Find the next object from the start address (dealing with alignment gaps, etc.) */
            let next: ObjectReference =
                unsafe { VM::VMObjectModel::get_object_from_start_address(current_object_end) };
            println!("next object: {}", next);
            /* Must be monotonically increasing */
            debug_assert!(next.to_address() > current.to_address());

            current = next;
        }
    }
}

impl<VM: VMBinding, PR: PageResource<VM>> Allocator<VM, PR> for BumpAllocator<VM, PR> {
    fn get_space(&self) -> Option<&'static PR::Space> {
        self.space
    }
    fn get_plan(&self) -> &'static SelectedPlan<VM> {
        self.plan
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        trace!("alloc");
        let result = align_allocation_no_fill(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            trace!("Thread local buffer used up, go to alloc slow path");
            self.alloc_slow(size, align, offset)
        } else {
            fill_alignment_gap(self.cursor, result);
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
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let acquired_start: Address = self
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
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }
}

impl<VM: VMBinding, PR: PageResource<VM>> BumpAllocator<VM, PR> {
    pub fn new(
        tls: OpaquePointer,
        space: Option<&'static PR::Space>,
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
}
