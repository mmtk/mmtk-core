use std::sync::Mutex;
use libc::c_void;
use ::space::Space;
use ::util::Address;
use ::util::align_allocation;

const BYTES_IN_PAGE: usize = 1 << 12;
const BLOCK_SIZE: usize = 8 * BYTES_IN_PAGE;
const BLOCK_MASK: usize = BLOCK_SIZE - 1;

lazy_static! {
    static ref IMMORTAL_SPACE: Mutex<Space> = Mutex::new(Space::new());
}

#[repr(C)]
#[derive(Debug)]
pub struct ThreadLocalAllocData {
    thread_id: usize,
    cursor: Address,
    limit: Address,
}

impl ThreadLocalAllocData {
    pub fn set_limit(&mut self, cursor: Address, limit: Address) {
        self.cursor = cursor;
        self.limit = limit;
    }

    pub fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let result = align_allocation(self.cursor, align, offset);
        let new_cursor = result + size;

        if new_cursor > self.limit {
            self.alloc_slow(size, align, offset)
        } else {
            self.cursor = new_cursor;
            result
        }
    }

    pub fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        let block_size = (size + BLOCK_MASK) & (!BLOCK_MASK);
        let mut space = IMMORTAL_SPACE.lock().unwrap();
        let acquired_start: Address = (*space).acquire(block_size);
        if acquired_start.is_zero() {
            acquired_start
        } else {
            self.set_limit(acquired_start, acquired_start + block_size);
            self.alloc(size, align, offset)
        }
    }
}

pub fn init(heap_size: usize) {
    let mut globl = IMMORTAL_SPACE.lock().unwrap();
    (*globl).init(heap_size);
}

pub fn bind_allocator(thread_id: usize) -> *mut ThreadLocalAllocData {
    Box::into_raw(Box::new(ThreadLocalAllocData {
        thread_id,
        cursor: unsafe { Address::zero() },
        limit: unsafe { Address::zero() },
    }))
}

pub fn alloc(handle: *mut ThreadLocalAllocData, size: usize,
             align: usize, offset: isize) -> *mut c_void {
    let local = unsafe { &mut *handle };
    local.alloc(size, align, offset).as_usize() as *mut c_void
}

pub fn alloc_slow(handle: *mut ThreadLocalAllocData, size: usize,
                  align: usize, offset: isize) -> *mut c_void {
    let local = unsafe { &mut *handle };
    local.alloc_slow(size, align, offset).as_usize() as *mut c_void
}
