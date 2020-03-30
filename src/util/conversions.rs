use crate::util::constants::*;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::Address;

/* Alignment */

pub fn is_address_aligned(addr: Address) -> bool {
    addr.is_aligned_to(BYTES_IN_ADDRESS)
}

pub fn page_align_down(address: Address) -> Address {
    address.align_down(BYTES_IN_PAGE)
}

pub fn is_page_aligned(address: Address) -> bool {
    address.is_aligned_to(BYTES_IN_PAGE)
}

// const function cannot have conditional expression
pub const fn chunk_align_up(addr: Address) -> Address {
    addr.align_up(BYTES_IN_CHUNK)
}

// const function cannot have conditional expression
pub const fn chunk_align_down(addr: Address) -> Address {
    addr.align_down(BYTES_IN_CHUNK)
}

pub const fn raw_align_up(val: usize, align: usize) -> usize {
    // See https://github.com/rust-lang/rust/blob/e620d0f337d0643c757bab791fc7d88d63217704/src/libcore/alloc.rs#L192
    val.wrapping_add(align).wrapping_sub(1) & !align.wrapping_sub(1)
}

pub const fn raw_align_down(val: usize, align: usize) -> usize {
    val & !align.wrapping_sub(1)
}

pub const fn raw_is_aligned(val: usize, align: usize) -> bool {
    val & align.wrapping_sub(1) == 0
}

/* Conversion */

pub fn pages_to_bytes(pages: usize) -> usize {
    pages << LOG_BYTES_IN_PAGE
}

pub fn bytes_to_pages_up(bytes: usize) -> usize {
    (bytes + BYTES_IN_PAGE - 1) >> LOG_BYTES_IN_PAGE
}

pub fn bytes_to_pages(bytes: usize) -> usize {
    let pages = bytes_to_pages_up(bytes);

    if cfg!(debug = "true") {
        let computed_extent = pages << LOG_BYTES_IN_PAGE;
        let bytes_match_pages = computed_extent == bytes;
        assert!(
            bytes_match_pages,
            "ERROR: number of bytes computed from pages must match original byte amount!\
             bytes = {}\
             pages = {}\
             bytes computed from pages = {}",
            bytes, pages, computed_extent
        );
    }

    pages
}

#[cfg(test)]
mod tests {
    use crate::util::conversions::*;
    use crate::util::Address;

    #[test]
    fn test_page_align() {
        let addr = unsafe { Address::from_usize(0x123456789) };
        assert_eq!(page_align_down(addr), unsafe {
            Address::from_usize(0x123456000)
        });
        assert!(!is_page_aligned(addr));
        assert!(is_page_aligned(page_align_down(addr)));
    }

    #[test]
    fn test_chunk_align() {
        let addr = unsafe { Address::from_usize(0x123456789) };
        assert_eq!(chunk_align_down(addr), unsafe {
            Address::from_usize(0x123400000)
        });
        assert_eq!(chunk_align_up(addr), unsafe {
            Address::from_usize(0x123800000)
        });
    }
}
