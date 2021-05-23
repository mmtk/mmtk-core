use libc::{c_void, size_t};

use crate::util::{Address, memory::dzmmap};

#[no_mangle]
pub extern "C" fn do_something() -> bool {
    eprintln!("Something");
    false
}

#[no_mangle]
pub extern "C" fn alloc_page() {
    // acquire 64kB space and return its address
}

#[no_mangle]
pub extern "C" fn mimalloc_dzmmap(start: *const c_void, size: size_t) {
    eprintln!("dzmmap");
    let addr = Address::from_ptr(start);
    match dzmmap(addr, size) {
        Ok(_) => {}
        Err(_) => {panic!()}
    }
}