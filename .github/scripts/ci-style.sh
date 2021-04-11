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
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    for_all_features "cargo clippy --target i686-unknown-linux-gnu"
    for_all_features "cargo clippy --release --target i686-unknown-linux-gnu"
fi 

# check format
cargo fmt -- --check
