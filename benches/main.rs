use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;

// As we can only initialize one MMTk instance, we have to run each benchmark in a separate process.
// So we only register one benchmark to criterion ('bench_main'), and based on the env var MMTK_BENCH,
// we pick the right benchmark to run.

// The benchmark can be executed with the following command. The feature `mock_test` is required, as the tests use MockVM.
// MMTK_BENCH=alloc cargo bench --features mock_test
// MMTK_BENCH=sft   cargo bench --features mock_test

// [Yi] I am not sure if these benchmarks are helpful any more after the MockVM refactoring. MockVM is really slow, as it
// is accessed with a lock, and it dispatches every call to function pointers in a struct. These tests may use MockVM,
// so they become slower as well. And the slowdown
// from MockVM may hide the actual performance difference when we change the functions that are benchmarked.
// We may want to improve the MockVM implementation so we can skip dispatching for benchmarking, or introduce another MockVM
// implementation for benchmarking.
// However, I will just keep these benchmarks here. If we find it not useful, and we do not plan to improve MockVM, we can delete
// them.

#[cfg(feature = "mock_test")]
mod mock_bench;

pub fn bench_main(_c: &mut Criterion) {
    #[cfg(feature = "mock_test")]
    match std::env::var("MMTK_BENCH") {
        Ok(bench) => match bench.as_str() {
            "alloc" => mock_bench::alloc::bench(_c),
            "internal_pointer" => mock_bench::internal_pointer::bench(_c),
            "sft" => mock_bench::sft::bench(_c),
            _ => panic!("Unknown benchmark {:?}", bench),
        },
        Err(_) => panic!("Need to name a benchmark by the env var MMTK_BENCH"),
    }

    #[cfg(not(feature = "mock_test"))]
    {
        eprintln!("ERROR: Currently there are no benchmarks when the \"mock_test\" feature is not enabled.");
        std::process::exit(1);
    }
}

criterion_group!(benches, bench_main);
criterion_main!(benches);
