set -xe

export RUSTFLAGS="-D warnings"
# check plan
cargo clippy --features nogc
cargo clippy --features semispace
# check features
cargo clippy --features nogc,sanity
cargo clippy --features semispace,sanity
cargo clippy --features nogc,vm_space,code_space,ro_space,lockfreeimmortalspace
cargo clippy --features semispace,vm_space,code_space,ro_space
# check for tests
cargo clippy --tests --features nogc
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features nogc
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features semispace
# check for different implementations of heap layout
cargo clippy --target i686-unknown-linux-gnu --features nogc
cargo clippy --target i686-unknown-linux-gnu --features nogc,force_32bit_heap_layout
cargo clippy --target x86_64-unknown-linux-gnu --features nogc
cargo clippy --target x86_64-unknown-linux-gnu --features nogc,force_32bit_heap_layout
# check format
cargo fmt -- --check
