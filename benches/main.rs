use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;

#[cfg(all(feature = "mock_test", feature = "test_private"))]
pub mod mock_bench;

#[cfg(all(not(feature = "mock_test"), feature = "test_private"))]
pub mod regular_bench;

pub fn bench_main(_c: &mut Criterion) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "mock_test")] {
            // If the "mock_test" feature is enabled, we only run mock test.
            mock_bench::bench(_c);
        } else if #[cfg(feature = "test_private")] {
            regular_bench::bench(_c);
        } else {
            eprintln!("ERROR: Benchmarks in mmtk_core requires the test_priavte feature (implied by mock_test) to run.");
            eprintln!("  Rerun with `MMTK_BENCH=\"bench_name\" cargo bench --features mock_test` to run mock-test benchmarks.");
            eprintln!("  Rerun with `cargo bench --features test_private -- bench_name` to run other benchmarks.");
            std::process::exit(1);
        }
    }
}

criterion_group!(benches, bench_main);
criterion_main!(benches);
