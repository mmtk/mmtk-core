use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::util::test_util::mock_vm::*;
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    {
        init_mockvm(MockVM {
            is_collection_enabled: MockMethod::new_fixed(Box::new(|_| false)),
            ..MockVM::default()
        });
    }
    // Setting a larger heap, although the GC should be disabled in the MockVM
    let fixture = MutatorFixture::create_with_heapsize(1 << 30);

    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr =
                memory_manager::alloc(fixture.mutator(), 8, 8, 0, AllocationSemantics::Default);
        })
    });
}
