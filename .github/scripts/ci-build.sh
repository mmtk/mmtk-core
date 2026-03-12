. $(dirname "$0")/ci-common.sh

# Execute this script under the root folder of this repo. Otherwise it will fail.

# Build basic
cargo build

# Build features
for_all_features "cargo build"

# Build release
for_all_features "cargo build --release"

# Build benches
for_all_features "cargo build --benches --features mock_test"

# target-specific features
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo build --features perf_counter
fi
