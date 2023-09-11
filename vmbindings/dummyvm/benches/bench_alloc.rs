use criterion::{criterion_group, Criterion};

use mmtk::plan::AllocationSemantics;
use mmtk_dummyvm::api;
use mmtk_dummyvm::test_fixtures::MutatorFixture;

fn alloc(c: &mut Criterion) {
    println!("Init MMTK in alloc bench");
    // 1GB so we are unlikely to OOM
    let fixture = MutatorFixture::create_with_heapsize(1 << 30);
    c.bench_function("alloc", |b| {
        b.iter(|| {
            let _addr = api::mmtk_alloc(fixture.mutator, 8, 8, 0, AllocationSemantics::Default);
        })
    });
}

criterion_group!(benches, alloc);
