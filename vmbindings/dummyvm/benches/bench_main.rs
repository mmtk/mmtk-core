use criterion::criterion_main;

mod sft;

criterion_main!{
    sft::benches,
}
