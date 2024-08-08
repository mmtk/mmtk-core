pub mod bscan;
pub mod bzero_bset;

pub use criterion::Criterion;

pub fn bench(c: &mut Criterion) {
    bscan::bench(c);
    bzero_bset::bench(c);
}
