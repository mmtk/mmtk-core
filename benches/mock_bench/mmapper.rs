pub use criterion::Criterion;

use mmtk::{memory_manager, util::test_util::fixtures::*};

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
}
