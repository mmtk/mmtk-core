use criterion::Criterion;
use mmtk::util::{
    constants::{BITS_IN_BYTE, BYTES_IN_WORD},
    metadata::side_metadata::{grain::AddressToBitAddress, SideMetadataSpec},
    Address,
};

pub fn bench(c: &mut Criterion) {
    let data_bytes = 256usize; // Match an Immix line size.
    let meta_bits = data_bytes / BYTES_IN_WORD;
    let meta_bytes = meta_bits / BITS_IN_BYTE;

    let allocate_u32 = || -> Address {
        let ptr = unsafe {
            std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(meta_bytes, 8).unwrap())
        };
        Address::from_mut_ptr(ptr)
    };

    let start = allocate_u32();
    let end = start + meta_bytes;
    let start_ba = start.with_bit_offset(0);
    let end_ba = end.with_bit_offset(0);

    c.bench_function("bzero_bset_modern_64", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, false, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, false, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_64_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, true, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, true, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_64_callback_normalize", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, true, true, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, true, true, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_64_callback_normalize_fastpath", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, true, true, true>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, true, true, true>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_32", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, false, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, false, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_32_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, true, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, true, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_32_callback_normalize", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, true, true, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, true, true, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_32_callback_normalize_fastpath", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, true, true, true>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, true, true, true>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_8", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u8, false, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u8, false, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_8_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u8, true, false, false>(
                start_ba, end_ba,
            );
            SideMetadataSpec::zero_meta_bits_modern_inner::<u8, true, false, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_8_callback_normalize", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u8, true, true, false>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u8, true, true, false>(
                start_ba, end_ba,
            );
        })
    });

    c.bench_function("bzero_bset_modern_8_callback_normalize_fastpath", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u8, true, true, true>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u8, true, true, true>(start_ba, end_ba);
        })
    });

    c.bench_function("bzero_bset_classic", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_classic::<false>(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits_classic::<false>(start, 0, end, 0);
        })
    });

    c.bench_function("bzero_bset_classic_fast", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_classic::<true>(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits_classic::<true>(start, 0, end, 0);
        })
    });
}
