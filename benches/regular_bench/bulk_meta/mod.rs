pub mod bzero_bset;

pub use criterion::Criterion;

pub fn bench(c: &mut Criterion) {
    bzero_bset::bench(c);
}
