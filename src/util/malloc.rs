use crate::util::constants::BYTES_IN_ADDRESS;
use crate::util::Address;
use crate::vm::VMBinding;
#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{calloc, free, malloc_usable_size, posix_memalign};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{
    mi_calloc as calloc, mi_calloc_aligned, mi_free as free,
    mi_malloc_usable_size as malloc_usable_size,
};

#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{calloc, free, malloc_usable_size};

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub use libc::{calloc, free, malloc_usable_size, posix_memalign};

#[cfg(not(any(feature = "malloc_mimalloc", feature = "malloc_hoard",)))]
fn align_alloc(size: usize, align: usize) -> Address {
    let mut ptr = std::ptr::null_mut::<libc::c_void>();
    let ptr_ptr = std::ptr::addr_of_mut!(ptr);
    let result = unsafe { posix_memalign(ptr_ptr, align, size) };
    if result != 0 {
        return Address::ZERO;
    }
    let address = Address::from_mut_ptr(ptr);
    crate::util::memory::zero(address, size);
    address
}

#[cfg(feature = "malloc_mimalloc")]
fn align_alloc(size: usize, align: usize) -> Address {
    let raw = unsafe { mi_calloc_aligned(1, size, align) };
    Address::from_mut_ptr(raw)
}

// hoard_sys does not provide align_alloc,
// we have to do it ourselves
#[cfg(feature = "malloc_hoard")]
fn align_alloc(size: usize, align: usize) -> Address {
    align_offset_alloc(size, align, 0)
}

// Beside returning the allocation result,
// this will store the malloc result at (result - BYTES_IN_ADDRESS)
fn align_offset_alloc<VM: VMBinding>(size: usize, align: usize, offset: isize) -> Address {
    // we allocate extra `align` bytes here, so we are able to handle offset
    let actual_size = size + align + BYTES_IN_ADDRESS;
    let raw = unsafe { calloc(1, actual_size) };
    let address = Address::from_mut_ptr(raw);
    if address.is_zero() {
        return address;
    }
    let mod_offset = (offset % (align as isize)) as isize;
    let mut result = crate::util::alloc::allocator::align_allocation_no_fill::<VM>(address, align, mod_offset); // address.add(1).align_up(align) - mod_offset;
    if result - BYTES_IN_ADDRESS < address {
        result += align;
    }
    let malloc_res_ptr: *mut usize = (result - BYTES_IN_ADDRESS).to_mut_ptr();
    unsafe { *malloc_res_ptr = address.as_usize() };
    result
}

fn offset_malloc_usable_size(address: Address) -> usize {
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { *malloc_res_ptr } as *mut libc::c_void;
    unsafe { malloc_usable_size(malloc_res) }
}

/// free an address that is allocated with some offset
pub fn offset_free(address: Address) {
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { *malloc_res_ptr } as *mut libc::c_void;
    unsafe { free(malloc_res) };
}

/// get malloc usable size of an address
/// is_offset_malloc: whether the address is allocated with some offset
pub fn get_malloc_usable_size(address: Address, is_offset_malloc: bool) -> usize {
    if is_offset_malloc {
        offset_malloc_usable_size(address)
    } else {
        unsafe { malloc_usable_size(address.to_mut_ptr()) }
    }
}

/// allocate `size` bytes, which is aligned to `align` at `offset`
/// return the address, and whether it is an offset allocation
pub fn alloc<VM: VMBinding>(size: usize, align: usize, offset: isize) -> (Address, bool) {
    let address: Address;
    let mut is_offset_malloc = false;
    // malloc returns 16 bytes aligned address.
    // So if the alignment is smaller than 16 bytes, we do not need to align.
    if align <= 16 && offset == 0 {
        let raw = unsafe { calloc(1, size) };
        address = Address::from_mut_ptr(raw);
        debug_assert!(address.is_aligned_to(align));
    } else if align > 16 && offset == 0 {
        address = align_alloc(size, align);
        #[cfg(feature = "malloc_hoard")]
        {
            is_offset_malloc = true;
        }
        debug_assert!(
            address.is_aligned_to(align),
            "Address: {:x} is not aligned to the given alignment: {}",
            address,
            align
        );
    } else {
        address = align_offset_alloc::<VM>(size, align, offset);
        is_offset_malloc = true;
        debug_assert!(
            (address + offset).is_aligned_to(align),
            "Address: {:x} is not aligned to the given alignment: {} at offset: {}",
            address,
            align,
            offset
        );
    }
    (address, is_offset_malloc)
}
