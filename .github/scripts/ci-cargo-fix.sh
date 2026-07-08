. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings -A unknown-lints"

# --- Fix main crate ---

# base
cargo fix --allow-dirty --allow-staged
cargo clippy --fix --allow-dirty --allow-staged

# all features
for_all_features "cargo fix --allow-dirty --allow-staged"
for_all_features "cargo clippy --fix --allow-dirty --allow-staged"

# --- Fix auxiliary crate ---

fix_auxiliary_crate() {
    crate_path=$1

    cargo fix --manifest-path=$crate_path/Cargo.toml --allow-dirty --allow-staged
    cargo clippy --fix --manifest-path=$crate_path/Cargo.toml --allow-dirty --allow-staged
}

fix_auxiliary_crate macros
fix_auxiliary_crate docs/dummyvm

# --- Format everything last, so the fixes above end up properly formatted ---

cargo fmt
cargo fmt --manifest-path=macros/Cargo.toml
cargo fmt --manifest-path=docs/dummyvm/Cargo.toml
