use crate::util::{constants, memory, Address};
use memory::dzmmap;
use std::io::{Error, Result};

use super::global::METADATA_SINGLETON;
use super::SideMetadataID;

#[cfg(target_pointer_width = "32")]
pub(super) const METADATA_BASE_ADDRESS: Address = unsafe { Address::from_usize(0) };
#[cfg(target_pointer_width = "64")]
pub(super) const METADATA_BASE_ADDRESS: Address =
    unsafe { Address::from_usize(0x0000_0600_0000_0000) };

#[cfg(target_pointer_width = "32")]
pub(super) const MAX_HEAP_SIZE_LOG: usize = 32;
// FIXME: This must be updated if the heap layout changes
#[cfg(target_pointer_width = "64")]
pub(super) const MAX_HEAP_SIZE_LOG: usize = 48;

pub(super) const MAX_METADATA_BITS: usize = constants::BITS_IN_WORD;
// const SPACE_PER_META_BIT: usize = 2 << (MAX_HEAP_SIZE_LOG - constants::LOG_BITS_IN_WORD);
pub(super) const META_SPACE_PAGE_SIZE: usize = constants::BYTES_IN_PAGE;
pub(super) const META_SPACE_PAGE_SIZE_LOG: usize = constants::LOG_BYTES_IN_PAGE as usize;

#[inline(always)]
pub(super) fn address_to_meta_address(addr: Address, metadata_id: SideMetadataID) -> Address {
    let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.as_usize()];
    // right shifts for `align` times, then
    // if bits_num_log < 3, right shift a few more times to cover multi objects per metadata byte
    // if bits_num_log = 3, metadata byte per object is 1
    // for > 3, left shift, because more than 1 byte per object is required
    let offset = (addr.as_usize() >> METADATA_SINGLETON.align[metadata_id.as_usize()])
        >> ((constants::LOG_BITS_IN_BYTE as usize) - bits_num_log);

    // info!("address_to_meta_address({}, {}).offset => 0x{:x}", addr, metadata_id.as_usize(), offset);
    METADATA_SINGLETON.meta_base_addr_vec[metadata_id.as_usize()] + offset
}

// Gets the related meta address and clears the low order bits
pub(super) fn address_to_meta_page_address(
    data_addr: Address,
    metadata_id: SideMetadataID,
) -> Address {
    let meta_addr = address_to_meta_address(data_addr, metadata_id);
    unsafe {
        Address::from_usize((meta_addr >> META_SPACE_PAGE_SIZE_LOG) << META_SPACE_PAGE_SIZE_LOG)
    }
}

// Checks whether the meta page containing this address is already mapped.
//
// Returns Err if the address is not mappable by mmtk,
// and Ok(is_mapped?) otherwise.
//
// NOTE: using incorrect (e.g. not properly aligned) page_addr is undefined behavior.
pub(super) fn meta_page_is_mapped(page_addr: Address) -> Result<bool> {
    let prot = libc::PROT_NONE;
    // MAP_FIXED_NOREPLACE returns EEXIST if already mapped
    let flags = libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
    let result: *mut libc::c_void = unsafe {
        libc::mmap(
            page_addr.to_mut_ptr(),
            META_SPACE_PAGE_SIZE,
            prot,
            flags,
            -1,
            0,
        )
    };
    if result == libc::MAP_FAILED {
        let err = unsafe { *libc::__errno_location() };
        if err == libc::EEXIST {
            // mmtk already mapped it
            Ok(true)
        } else {
            // mmtk can't map it
            Err(Error::from_raw_os_error(err as _))
        }
    } else {
        // mmtk can map it
        // first, unmap the mapped memory
        let result2 = unsafe { libc::munmap(page_addr.to_mut_ptr(), META_SPACE_PAGE_SIZE) };
        assert_ne!(result2, libc::MAP_FAILED as _);
        Ok(false)
    }
}

fn find_middle_page(first_page: Address, last_page: Address) -> Address {
    let total_page_num = (last_page.as_usize() + META_SPACE_PAGE_SIZE - first_page.as_usize())
        / META_SPACE_PAGE_SIZE;
    first_page + (META_SPACE_PAGE_SIZE * (total_page_num / 2))
}

pub(super) fn ensure_meta_is_mapped(
    start: Address,
    size: usize,
    metadata_id: SideMetadataID,
) -> bool {
    let last_meta_page = address_to_meta_page_address(start + size - 1, metadata_id);
    if meta_page_is_mapped(last_meta_page).unwrap() {
        // all required pages are already mapped
        return true;
    }
    let first_meta_page = address_to_meta_page_address(start, metadata_id);
    if !meta_page_is_mapped(first_meta_page).unwrap() {
        // map the whole area
        if let Err(e) = dzmmap(
            first_meta_page,
            last_meta_page.as_usize() - first_meta_page.as_usize() + META_SPACE_PAGE_SIZE,
        ) {
            debug!(
                "ensure_meta_is_mapped failed to map the required meta space with error: {}",
                e
            );
            return false;
        }
        return true;
    }
    // find the first to be mapped page, and map from there onwards
    //
    // Here, we know the first_meta_page is mapped and the last is not.
    // The following loop performs a binary search.
    // At the end of the loop, both middle_page and last_page contain the result
    let mut first_page = first_meta_page;
    let mut last_page = last_meta_page;
    let mut middle_page = find_middle_page(first_page, last_page);
    while middle_page != last_page {
        if meta_page_is_mapped(middle_page).unwrap() {
            first_page = middle_page;
        } else {
            last_page = middle_page;
        }
        middle_page = find_middle_page(first_page, last_page);
    }

    if let Err(e) = dzmmap(
        middle_page,
        size - (middle_page.as_usize() - first_meta_page.as_usize()),
    ) {
        debug!(
            "ensure_meta_is_mapped failed to map the required meta space with error: {}",
            e
        );
        return false;
    }

    true
}

#[inline(always)]
pub(super) fn meta_space_size(metadata_id: SideMetadataID) -> usize {
    let actual_size = 1usize
        << (MAX_HEAP_SIZE_LOG
            - constants::LOG_BITS_IN_WORD
            - METADATA_SINGLETON.align[metadata_id.as_usize()]
            + METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.as_usize()]);
    // final size is always a multiple of page size
    round_up_to_page_size(actual_size)
}

#[inline(always)]
pub(super) fn round_up_to_page_size(size: usize) -> usize {
    if size % META_SPACE_PAGE_SIZE == 0 {
        size
    } else {
        // round-up the size to page size
        ((size >> constants::LOG_BYTES_IN_PAGE) + 1) << constants::LOG_BITS_IN_PAGE
    }
}

#[inline(always)]
pub(super) fn meta_byte_lshift(addr: Address, metadata_id: SideMetadataID) -> usize {
    // I assume compilers are smart enough to optimize remainder to (2^n) operations
    let bits_num_log = METADATA_SINGLETON.meta_bits_num_log_vec[metadata_id.as_usize()];
    ((addr.as_usize() >> constants::LOG_BYTES_IN_WORD) % (constants::BITS_IN_BYTE >> bits_num_log))
        << bits_num_log
}
