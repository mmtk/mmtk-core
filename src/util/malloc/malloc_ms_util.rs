use crate::util::constants::BYTES_IN_ADDRESS;
use crate::util::malloc::library::*;
use crate::util::Address;
use crate::vm::VMBinding;

/// Allocate with alignment. This also guarantees the memory is zero initialized.
pub fn align_alloc<VM: VMBinding>(size: usize, align: usize) -> Address {
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

/// Allocate with alignment and offset.
/// Beside returning the allocation result, this will store the malloc result at (result - BYTES_IN_ADDRESS)
/// so we know the original malloc result.
pub fn align_offset_alloc<VM: VMBinding>(size: usize, align: usize, offset: usize) -> Address {
    // we allocate extra `align` bytes here, so we are able to handle offset
    let actual_size = size + align + BYTES_IN_ADDRESS;
    let raw = unsafe { calloc(1, actual_size) };
    let address = Address::from_mut_ptr(raw);
    if address.is_zero() {
        return address;
    }
    let mod_offset = offset % align;
    let mut result =
        crate::util::alloc::allocator::align_allocation_no_fill::<VM>(address, align, mod_offset);
    if result - BYTES_IN_ADDRESS < address {
        result += align;
    }
    let malloc_res_ptr: *mut usize = (result - BYTES_IN_ADDRESS).to_mut_ptr();
    unsafe { malloc_res_ptr.write_unaligned(address.as_usize()) };
    result
}

/// Get the malloc usable size for an address that is returned by [`crate::util::malloc::malloc_ms_util::align_offset_alloc`].
pub fn offset_malloc_usable_size(address: Address) -> usize {
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { malloc_res_ptr.read_unaligned() } as *mut libc::c_void;
    unsafe { malloc_usable_size(malloc_res) }
}

/// Free an address that is allocated with an offset (returned by [`crate::util::malloc::malloc_ms_util::align_offset_alloc`]).
pub fn offset_free(address: Address) {
    let malloc_res_ptr: *mut usize = (address - BYTES_IN_ADDRESS).to_mut_ptr();
    let malloc_res = unsafe { malloc_res_ptr.read_unaligned() } as *mut libc::c_void;
    unsafe { free(malloc_res) };
}

pub use crate::util::malloc::library::free;

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
pub fn alloc<VM: VMBinding>(size: usize, align: usize, offset: usize) -> (Address, bool) {
    let address: Address;
    let mut is_offset_malloc = false;
    // malloc returns 16 bytes aligned address.
    // So if the alignment is smaller than 16 bytes, we do not need to align.
    if align <= 16 && offset == 0 {
        let raw = unsafe { calloc(1, size) };
        address = Address::from_mut_ptr(raw);
        debug_assert!(address.is_aligned_to(align));
    } else if align > 16 && offset == 0 {
        address = align_alloc::<VM>(size, align);
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
