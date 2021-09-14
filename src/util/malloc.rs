#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{calloc, free, malloc_usable_size};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{
    mi_calloc as calloc, mi_free as free, mi_malloc_usable_size as malloc_usable_size,
};

#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{calloc, free, malloc_usable_size};

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub use libc::{calloc, free, malloc_usable_size, posix_memalign};
use crate::util::Address;
use crate::util::constants::BYTES_IN_ADDRESS;
use crate::util::memory;

pub fn alloc(size: usize) -> Address {
    let raw = unsafe { calloc(1, size) };
    let address = Address::from_mut_ptr(raw);
    address
}

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub fn align_alloc(size: usize, align: usize) -> Address {
    let mut ptr = 0 as usize as *mut libc::c_void;
    let ptr_ptr = std::ptr::addr_of_mut!(ptr);
    let result = unsafe { posix_memalign(ptr_ptr, align, size) };
    if result != 0 {
        return Address::ZERO;
    }
    let address = Address::from_mut_ptr(ptr);
    memory::zero(address, size);
    address
}

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub fn align_offset_alloc(size: usize, align: usize, offset: isize) -> Address {
    let actual_size = size + align + BYTES_IN_ADDRESS;
    let address = align_alloc(actual_size, align);
    if address.is_zero() {
        return address;
    }
    let mod_offset = offset as usize % align;
    let mut result = address + align - mod_offset;
    if mod_offset + BYTES_IN_ADDRESS > align {
        result += align;
    }
    let malloc_res_ptr: *mut usize = (result - BYTES_IN_ADDRESS).to_mut_ptr();
    unsafe { *malloc_res_ptr = address.as_usize() };
    result
}

pub fn offset_malloc_usable_size(address: Address) -> usize {
    // let address = Address.from_mut_ptr(ptr);
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { *malloc_res_ptr } as *mut libc::c_void;
    unsafe { malloc_usable_size(malloc_res) }
}

pub fn offset_free(address: Address) {
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { *malloc_res_ptr } as *mut libc::c_void;
    unsafe { free(malloc_res) };
}