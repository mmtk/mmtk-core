use ::util::Address;
use ::util::heap::layout::vm_layout_constants::*;
use ::util::constants::*;

pub fn page_align(address: Address) -> Address {
    Address((address.0 >> LOG_BYTES_IN_PAGE) << LOG_BYTES_IN_PAGE)
}

pub fn is_page_aligned(address: Address) -> bool {
    page_align(address) == address
}

pub fn align_word(mut addr: usize, bits: usize, down: bool) -> usize {
    if !down {
      if BITS_IN_ADDRESS == 64 && bits >= 32 {
        debug_assert!(bits < 64);
        addr = addr + ((1usize << bits) - 1);
      } else {
        debug_assert!(bits < 32);
        addr = addr + ((1 << bits) - 1);
      }
    }
    (addr >> bits) << bits
}

pub fn align_up(addr: Address, bits: usize) -> Address {
    Address(align_word(addr.0, bits, false))
}

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