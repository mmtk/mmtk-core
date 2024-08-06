use criterion::Criterion;

mod bulk_meta;

pub fn bench(c: &mut Criterion) {
    bulk_meta::bench(c);
}
