use criterion::{criterion_group, Criterion};

use mmtk::plan::AllocationSemantics;
use mmtk::vm::ObjectModel;
use mmtk_dummyvm::api;
use mmtk_dummyvm::test_fixtures::MutatorFixture;

fn post_alloc(c: &mut Criterion) {
    // 1GB so we are unlikely to OOM
    let fixture = MutatorFixture::create_with_heapsize(1 << 30);
    let addr = api::mmtk_alloc(fixture.mutator, 16, 4, 0, AllocationSemantics::Default);
    let obj = mmtk_dummyvm::object_model::VMObjectModel::address_to_ref(addr);
    c.bench_function("post_alloc", |b| {
        b.iter(|| {
            api::mmtk_post_alloc(fixture.mutator, obj, 8, AllocationSemantics::Default);
        })
    });
}

criterion_group!(benches, post_alloc);
