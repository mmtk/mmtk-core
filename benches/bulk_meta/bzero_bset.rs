//! Benchmarks for bulk zeroing and setting.

use std::os::raw::c_void;

use criterion::Criterion;
use mmtk::util::{constants::LOG_BITS_IN_WORD, metadata::side_metadata::SideMetadataSpec, Address};

fn allocate_aligned(size: usize) -> Address {
    let ptr = unsafe {
        std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(size, size).unwrap())
    };
    Address::from_mut_ptr(ptr)
}

const LINE_BYTES: usize = 256usize; // Match an Immix line size.
const BLOCK_BYTES: usize = 32768usize; // Match an Immix block size.

// Asssume one-bit-per-word metadata (matching VO bits).
const LINE_META_BYTES: usize = LINE_BYTES >> LOG_BITS_IN_WORD;
const BLOCK_META_BYTES: usize = BLOCK_BYTES >> LOG_BITS_IN_WORD;

pub fn bench(c: &mut Criterion) {
    c.bench_function("bzero_bset_line", |b| {
        let start = allocate_aligned(LINE_META_BYTES);
        let end = start + LINE_META_BYTES;

        b.iter(|| {
            SideMetadataSpec::bench_set_meta_bits(start, 0, end, 0);
            SideMetadataSpec::bench_zero_meta_bits(start, 0, end, 0);
        })
    });

    c.bench_function("bzero_bset_line_memset", |b| {
        let start = allocate_aligned(LINE_META_BYTES);
        let end = start + LINE_META_BYTES;

        b.iter(|| unsafe {
            libc::memset(start.as_mut_ref() as *mut c_void, 0xff, end - start);
            libc::memset(start.as_mut_ref() as *mut c_void, 0x00, end - start);
        })
    });

    c.bench_function("bzero_bset_block", |b| {
        let start = allocate_aligned(BLOCK_META_BYTES);
        let end = start + BLOCK_META_BYTES;

        b.iter(|| {
            SideMetadataSpec::bench_set_meta_bits(start, 0, end, 0);
            SideMetadataSpec::bench_zero_meta_bits(start, 0, end, 0);
        })
    });

    c.bench_function("bzero_bset_block_memset", |b| {
        let start = allocate_aligned(BLOCK_META_BYTES);
        let end = start + BLOCK_META_BYTES;

        b.iter(|| unsafe {
            libc::memset(start.as_mut_ref() as *mut c_void, 0xff, end - start);
            libc::memset(start.as_mut_ref() as *mut c_void, 0x00, end - start);
        })
    });
}
