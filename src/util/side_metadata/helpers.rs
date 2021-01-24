use crate::util::{constants, conversions, memory, Address};
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
pub(super) const MAX_HEAP_SIZE_LOG: usize = 31;
// FIXME: This must be updated if the heap layout changes
#[cfg(target_pointer_width = "64")]
pub(super) const MAX_HEAP_SIZE_LOG: usize = 46;

// This is the maximum number of bits in a metadata bit-set
pub(super) const MAX_METADATA_BITS: usize = constants::BITS_IN_WORD;
// This is the maximum number of metadata bit-sets in an MMTk instance
pub(super) const MAX_METADATA_ID: usize = constants::BITS_IN_WORD;

// const SPACE_PER_META_BIT: usize = 2 << (MAX_HEAP_SIZE_LOG - constants::LOG_BITS_IN_WORD);
pub(super) const META_SPACE_PAGE_SIZE: usize = constants::BYTES_IN_PAGE;

#[inline(always)]
pub(super) fn address_to_meta_address(addr: Address, metadata_id: SideMetadataID) -> Address {
    debug_assert!(
        metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
        "metadata_id ({}) out of range",
        metadata_id.as_usize()
    );
    let bits_num_log = unsafe {
        METADATA_SINGLETON
            .meta_bits_num_log_vec
            .get_unchecked(metadata_id.as_usize())
    };
    let bits_num_log = *bits_num_log as i32;
    // right shifts for `align` times, then
    // if bits_num_log < 3, right shift a few more times to cover multi objects per metadata byte
    // if bits_num_log = 3, metadata byte per object is 1
    // for > 3, left shift, because more than 1 byte per object is required
    let rshift = (constants::LOG_BITS_IN_BYTE as i32) - bits_num_log;
    let offset = unsafe {
        if rshift >= 0 {
            addr.as_usize()
                >> (*METADATA_SINGLETON
                    .align
                    .get_unchecked(metadata_id.as_usize()) as u32)
                >> (rshift as u32)
        } else {
            addr.as_usize()
                >> (*METADATA_SINGLETON
                    .align
                    .get_unchecked(metadata_id.as_usize()) as u32)
                << (-rshift as u32)
        }
    };

    unsafe {
        *METADATA_SINGLETON
            .meta_base_addr_vec
            .get_unchecked(metadata_id.as_usize())
            + offset
    }
}

// Gets the related meta address and clears the low order bits
pub(super) fn address_to_meta_page_address(
    data_addr: Address,
    metadata_id: SideMetadataID,
) -> Address {
    conversions::page_align_down(address_to_meta_address(data_addr, metadata_id))
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

pub(super) fn try_map_meta(start: Address, size: usize, metadata_id: SideMetadataID) -> bool {
    let last_meta_page = address_to_meta_page_address(start + size - 1, metadata_id);
    match meta_page_is_mapped(last_meta_page) {
        Ok(is_mapped) => {
            if is_mapped {
                // all required pages are already mapped -> success
                return true;
            }
        }
        Err(_) => {
            // (at least) the last page is not mappable -> failure
            return false;
        }
    }
    let first_meta_page = address_to_meta_page_address(start, metadata_id);
    match meta_page_is_mapped(first_meta_page) {
        Ok(is_mapped) => {
            // first page is not mapped yet -> try mapping the whole area
            if !is_mapped {
                // map the whole area
                if let Err(e) = dzmmap(
                    first_meta_page,
                    last_meta_page.as_usize() - first_meta_page.as_usize() + META_SPACE_PAGE_SIZE,
                ) {
                    debug!(
                        "try_map_meta failed to map the required meta space with error: {}",
                        e
                    );
                    return false;
                }
                return true;
            }
            // first page is already mapped and last page is not
        }
        Err(_) => {
            // (at least) the first page is not mappable -> failure
            return false;
        }
    }
    // // find the first to be mapped page, and map from there onwards
    // //
    // // Here, we know the first_meta_page is mapped and the last is not.
    // // The following loop performs a binary search.
    // // At the end of the loop, both middle_page and last_page contain the result
    // let mut first_page = first_meta_page;
    // let mut last_page = last_meta_page;
    // let mut middle_page = find_middle_page(first_page, last_page);
    // while middle_page != last_page {
    //     match meta_page_is_mapped(middle_page) {
    //         Ok(is_mapped) => {
    //             if is_mapped {
    //                 first_page = middle_page;
    //             } else {
    //                 last_page = middle_page;
    //             }
    //         }
    //         Err(_) => {
    //             // non-mappable page detected -> failure
    //             return false;
    //         }
    //     }
    //     middle_page = find_middle_page(first_page, last_page);
    // }

    // Considering that this function is only called on space growth,
    // there is zero or one mapped meta page in the range.
    // If we were to support space shrink, we needed a binary search,
    // because there could be more than one mapped meta page.
    if let Err(e) = dzmmap(
        first_meta_page + META_SPACE_PAGE_SIZE,
        size - META_SPACE_PAGE_SIZE,
    ) {
        debug!(
            "try_map_meta failed to map the required meta space with error: {}",
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
    conversions::raw_align_up(size, META_SPACE_PAGE_SIZE)
}

#[inline(always)]
pub(super) fn meta_byte_lshift(addr: Address, metadata_id: SideMetadataID) -> usize {
    // I assume compilers are smart enough to optimize remainder to (2^n) operations
    debug_assert!(
        metadata_id.as_usize() < METADATA_SINGLETON.meta_bits_num_log_vec.len(),
        "metadata_id ({}) out of range",
        metadata_id.as_usize()
    );
    let bits_num_log = unsafe {
        METADATA_SINGLETON
            .meta_bits_num_log_vec
            .get_unchecked(metadata_id.as_usize())
    };
    ((addr.as_usize() >> constants::LOG_BYTES_IN_WORD) % (constants::BITS_IN_BYTE >> bits_num_log))
        << bits_num_log
}

#[cfg(test)]
mod tests {
    use crate::util::side_metadata::helpers::*;

    #[test]
    fn test_side_metadata_helpers_round_up_to_page_size() {
        assert_eq!(round_up_to_page_size(1), META_SPACE_PAGE_SIZE);
        assert_eq!(
            round_up_to_page_size(META_SPACE_PAGE_SIZE - 1),
            META_SPACE_PAGE_SIZE
        );
        assert_eq!(
            round_up_to_page_size(META_SPACE_PAGE_SIZE),
            META_SPACE_PAGE_SIZE
        );
        assert_eq!(
            round_up_to_page_size(META_SPACE_PAGE_SIZE + 1),
            META_SPACE_PAGE_SIZE << 1
        );
    }
}
