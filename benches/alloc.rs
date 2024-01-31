use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::util::test_util::mock_method::*;
use mmtk::util::test_util::mock_vm::{write_mockvm, MockVM};
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    // Setting a larger heap (1TB) so we won't trigger GC, but we should disable GC if we can
    let mut fixture = MutatorFixture::create_with_heapsize(1 << 40);
    {
        write_mockvm(|mock| {
            *mock = {
                MockVM {
                    is_collection_enabled: MockMethod::new_fixed(Box::new(|_| false)),
                    ..MockVM::default()
                }
            }
        });
    }

    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr =
                memory_manager::alloc(&mut fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        })
    });
}
