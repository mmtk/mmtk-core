use ::util::Address;
use ::util::heap::layout::vm_layout_constants::*;
use ::util::constants::*;

#[macro_export]
macro_rules! chunk_align {
    ($addr:expr, $down:expr) => (
        (if_then_else_usize!($down, $addr, $addr +
            ::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK - 1) >>
                ::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK)
                    << ::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK
    );
}

pub fn chunk_align(immut_addr: Address, down: bool) -> Address {
    let addr = if !down { immut_addr + BYTES_IN_CHUNK - 1 } else { immut_addr };
    unsafe {
        Address::from_usize((addr.as_usize() >> LOG_BYTES_IN_CHUNK) << LOG_BYTES_IN_CHUNK)
    }
}

pub fn raw_chunk_align(immut_addr: usize, down: bool) -> usize {
    let addr = if !down { immut_addr + BYTES_IN_CHUNK - 1 } else { immut_addr };
    (addr >> LOG_BYTES_IN_CHUNK) << LOG_BYTES_IN_CHUNK
}

pub fn pages_to_bytes(pages: usize) -> usize {
    pages << LOG_BYTES_IN_PAGE
}

pub fn bytes_to_pages_up(bytes: usize) -> usize {
    (bytes + BYTES_IN_PAGE - 1) >> LOG_BYTES_IN_PAGE
}

pub fn bytes_to_pages(bytes: usize) -> usize {
    let pages = bytes_to_pages_up(bytes);

    if cfg!(debug = "true") {
        let computed_extent = pages_to_address(pages);
        let bytes_match_pages = computed_extent.as_usize() == bytes;
        assert!(bytes_match_pages, "ERROR: number of bytes computed from pages must match original byte amount!\
                                           bytes = {}\
                                           pages = {}\
                                           bytes computed from pages = {}", bytes, pages, computed_extent);
    }

    pages
}

pub fn pages_to_address(pages: usize) -> Address {
    unsafe{Address::from_usize(pages << LOG_BYTES_IN_PAGE)}
}