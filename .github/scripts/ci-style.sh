. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings -A unknown-lints"

# check base
cargo clippy
# check all features
for_all_features "cargo clippy"
# check release
for_all_features "cargo clippy --release"
# check for tests
for_all_features "cargo clippy --tests"
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml

# target-specific features
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo clippy --features perf_counter
    cargo clippy --release --features perf_counter
    cargo clippy --tests --features perf_counter
fi

# check format
cargo fmt -- --check
