pub use criterion::Criterion;

use mmtk::{
    memory_manager, mmap_anno_test,
    util::{constants::BYTES_IN_PAGE, memory::MmapStrategy, test_util::fixtures::*, Address},
};

pub fn bench(c: &mut Criterion) {
    let mut fixture = MutatorFixture::create_with_heapsize(1 << 30);

    let regular = memory_manager::alloc(
        &mut fixture.mutator,
        40,
        0,
        0,
        mmtk::AllocationSemantics::Default,
    );

    let large = memory_manager::alloc(
        &mut fixture.mutator,
        40,
        0,
        0,
        mmtk::AllocationSemantics::Los,
    );

    let low = unsafe { Address::from_usize(42usize) };
    let high = unsafe { Address::from_usize(usize::MAX - 1024usize) };

    c.bench_function("is_mapped_regular", |b| {
        b.iter(|| {
            let is_mapped = regular.is_mapped();
            assert!(is_mapped);
        })
    });

    c.bench_function("is_mapped_large", |b| {
        b.iter(|| {
            let is_mapped = large.is_mapped();
            assert!(is_mapped);
        })
    });

    c.bench_function("is_mapped_low", |b| {
        b.iter(|| {
            let is_mapped = low.is_mapped();
            assert!(!is_mapped);
        })
    });

    c.bench_function("is_mapped_high", |b| {
        b.iter(|| {
            let is_mapped = high.is_mapped();
            assert!(!is_mapped);
        })
    });

    // The following bench involves large address ranges and cannot run on 32-bit machines.
    #[cfg(target_pointer_width = "64")]
    c.bench_function("is_mapped_seq", |b| {
        b.iter(|| {
            use mmtk::util::heap::vm_layout::BYTES_IN_CHUNK;
            let start = regular.as_usize();
            let num_chunks = 16384usize;
            let end = start + num_chunks * BYTES_IN_CHUNK;
            for addr_usize in (start..end).step_by(BYTES_IN_CHUNK) {
                let addr = unsafe { Address::from_usize(addr_usize) };
                let _is_mapped = addr.is_mapped();
            }
        })
    });

    c.bench_function("ensure_mapped_regular", |b| {
        let start = regular.align_down(BYTES_IN_PAGE);
        assert!(start.is_mapped());
        let strategy = MmapStrategy::new(false, mmtk::util::memory::MmapProtection::ReadWrite);
        let anno = mmap_anno_test!();
        b.iter(|| {
            mmtk::MMAPPER
                .ensure_mapped(start, 1, strategy, anno)
                .unwrap();
        })
    });
}
