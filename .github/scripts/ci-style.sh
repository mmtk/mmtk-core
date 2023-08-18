. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings"

# check base
cargo clippy
# check all features
for_all_features "cargo clippy"
# check release
for_all_features "cargo clippy --release"
# check for tests
for_all_features "cargo clippy --tests"

# target-specific features
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo clippy --features perf_counter
    cargo clippy --release --features perf_counter
    cargo clippy --tests --features perf_counter
fi

style_check_auxiliary_crate() {
    crate_path = $1

    cargo clippy --manifest-path=$crate_path/Cargo.toml
    cargo fmt --manifest-path=$crate_path/Cargo.toml -- --check
}

style_check_auxiliary_crate macros
style_check_auxiliary_crate vmbindings/dummyvm
