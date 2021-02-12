. $(dirname "$0")/ci-common.sh

# Execute this script under the root folder of this repo. Otherwise it will fail.

# Build basic
cargo build

# Build features
for_all_features "cargo build"

# For x86_64-linux, also see if we can build for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo build --target i686-unknown-linux-gnu
    for_all_features "cargo build --target i686-unknown-linux-gnu"
fi