set -xe

export RUSTFLAGS="-D warnings"
# check plan
cargo clippy --features nogc
cargo clippy --features semispace
# check features
cargo clippy --features nogc,sanity
cargo clippy --features semispace,sanity
cargo clippy --features nogc,vmspace
cargo clippy --features semispace,vmspace
# check for tests
cargo clippy --tests --features nogc
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features nogc
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features semispace
cargo fmt -- --check
