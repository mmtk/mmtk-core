use criterion::Criterion;
use mmtk::util::{metadata::side_metadata::SideMetadataSpec, Address};

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

    c.bench_function("bzero_bset_modern", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits(start, 0, end, 0);
        })
    });

    c.bench_function("bzero_bset_modern_callback", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_callback(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits_callback(start, 0, end, 0);
        })
    });

    c.bench_function("bzero_bset_classic", |b| {
        b.iter(|| {
            SideMetadataSpec::set_meta_bits_classic(start, 0, end, 0);
            SideMetadataSpec::zero_meta_bits_classic(start, 0, end, 0);
        })
    });
}
