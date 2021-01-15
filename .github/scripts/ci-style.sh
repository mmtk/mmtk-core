set -xe

. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings"

# check base
cargo clippy
# check all features
cargo clippy --all-features
# check for tests
cargo clippy --tests --all-features
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --all-features

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo clippy --target i686-unknown-linux-gnu --all-features
fi 

# check format
cargo fmt -- --check
