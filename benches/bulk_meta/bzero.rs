use criterion::Criterion;
use mmtk::util::{
    metadata::side_metadata::{grain::AddressToBitAddress, SideMetadataSpec},
    Address,
};

pub fn bench(c: &mut Criterion) {
    let size = 256usize; // Match an Immix line size.

    let allocate_u32 = || -> Address {
        let ptr = unsafe {
            std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(size, 8).unwrap())
        };
        Address::from_mut_ptr(ptr)
    };

    let start = allocate_u32();
    let end = start + size;
    let start_ba = start.with_bit_offset(0);
    let end_ba = end.with_bit_offset(0);

    c.bench_function("bzero_bset_modern_64", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, false>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, false>(start_ba, end_ba);
        })
    });

    c.bench_function("bzero_bset_modern_64_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u64, true>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u64, true>(start_ba, end_ba);
        })
    });

    c.bench_function("bzero_bset_modern_32", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, false>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, false>(start_ba, end_ba);
        })
    });

    c.bench_function("bzero_bset_modern_32_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_modern_inner::<u32, true>(start_ba, end_ba);
            SideMetadataSpec::zero_meta_bits_modern_inner::<u32, true>(start_ba, end_ba);
        })
    });

    c.bench_function("bzero_bset_classic", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_classic(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits_classic(start, 0, end, 0);
        })
    });
}
