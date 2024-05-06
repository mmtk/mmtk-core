. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings -A unknown-lints"

# --- Check format ---
cargo fmt -- --check
cargo fmt --manifest-path=macros/Cargo.toml -- --check

# All versions of Clippy randomly crash on Darwin.  We disable Clippy tests for Darwin for now.
if [[ $(uname) == "Darwin" ]]; then
    exit 0
fi

# Workaround the clippy issue on Rust 1.72: https://github.com/mmtk/mmtk-core/issues/929.
# If we are not testing with Rust 1.72, or there is no problem running the following clippy checks, we can remove this export.
CLIPPY_VERSION=$(cargo clippy --version)
if [[ $CLIPPY_VERSION == "clippy 0.1.72"* ]]; then
    export CARGO_INCREMENTAL=0
fi

# --- Check main crate ---

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

# mock tests
cargo clippy --features mock_test
cargo clippy --features mock_test --tests
cargo clippy --features mock_test --benches

# --- Check auxiliary crate ---

style_check_auxiliary_crate() {
    crate_path=$1

    cargo clippy --manifest-path=$crate_path/Cargo.toml
    cargo fmt --manifest-path=$crate_path/Cargo.toml -- --check
}

style_check_auxiliary_crate macros
