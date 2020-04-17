set -xe

export RUSTFLAGS="-D warnings"
cargo clippy --features nogc
cargo clippy --features semispace
cargo clippy --features nogc,sanity
cargo clippy --features semispace,sanity
cargo clippy --features nogc,vmspace
cargo clippy --features semispace,vmspace
cargo fmt -- --check
