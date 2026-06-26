use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    // Setting a larger heap, although the GC should be disabled in the MockVM
    let mut fixture = MutatorFixture::create_with_heapsize(1 << 30);
    fixture.mmtk().disable_collection();

    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr =
                memory_manager::alloc(&mut fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        })
    });
}
