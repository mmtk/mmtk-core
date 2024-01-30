use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    // Setting a larger heap so we won't trigger GC, but we should disable GC if we can
    let mut fixture = MutatorFixture::create_with_heapsize(usize::MAX);
    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr =
                memory_manager::alloc(&mut fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        })
    });
}
