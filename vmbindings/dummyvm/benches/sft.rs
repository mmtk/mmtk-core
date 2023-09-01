use criterion::{black_box, criterion_group, Criterion};

use mmtk_dummyvm::api;
use mmtk_dummyvm::test_fixtures::MutatorFixture;
use mmtk_dummyvm::test_fixtures::FixtureContent;
use mmtk::plan::AllocationSemantics;
use mmtk::vm::ObjectModel;

fn sft_bench(c: &mut Criterion) {
    let fixture = MutatorFixture::create();
    let addr = api::mmtk_alloc(fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
    let obj = mmtk_dummyvm::object_model::VMObjectModel::address_to_ref(addr);

    c.bench_function("sft read", |b| b.iter(|| {
        api::mmtk_is_in_mmtk_spaces(black_box(obj))
    }));
}

criterion_group!(benches, sft_bench);
