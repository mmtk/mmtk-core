use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    // Disable GC so we won't trigger GC
    let mut fixture = MutatorFixture::create_with_heapsize(1 << 30);
    memory_manager::disable_collection(fixture.mmtk());
    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr =
                memory_manager::alloc(&mut fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        })
    });
}
