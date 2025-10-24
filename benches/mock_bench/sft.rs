use criterion::black_box;
use criterion::Criterion;

use mmtk::memory_manager;
use mmtk::util::test_util::fixtures::*;
use mmtk::util::test_util::mock_vm::*;
use mmtk::AllocationSemantics;

pub fn bench(c: &mut Criterion) {
    let mut fixture = MutatorFixture::create();
    let addr = memory_manager::alloc(fixture.mutator(), 8, 8, 0, AllocationSemantics::Default);
    let obj = MockVM::object_start_to_ref(addr);

    c.bench_function("sft read", |b| {
        b.iter(|| memory_manager::is_in_mmtk_spaces(black_box(obj)))
    });
}
