use criterion::criterion_main;

// As we can only initialize one MMTk instance, we have to run each benchmark separately.
// Filtering like `cargo bench -- sft` won't work, as it still evalutes all the benchmark groups (which initialize MMTk).
// We can use conditional compilation, and run with `cargo bench --features bench_sft`. The features are defined in the dummy VM crate.

#[cfg(feature = "bench_sft")]
mod bench_sft;
#[cfg(feature = "bench_sft")]
criterion_main!(bench_sft::benches);

#[cfg(feature = "bench_alloc")]
mod bench_alloc;
#[cfg(feature = "bench_alloc")]
criterion_main!(bench_alloc::benches);
