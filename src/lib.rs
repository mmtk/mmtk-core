mod address;

use address::Address;

#[no_mangle]
pub extern fn gc_init() {}

#[no_mangle]
pub extern fn alloc(size: usize, align: usize) -> Address {
    unsafe {Address::zero()}
}