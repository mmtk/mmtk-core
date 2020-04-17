set -xe

# Execute this script under the root folder of this repo. Otherwise it will fail.

# Build plans
cargo build --features nogc
cargo build --features semispace

# Build features
cargo build --features nogc,vmspace
cargo build --features semispace,vmspace
cargo build --features nogc,sanity
cargo build --features semispace,sanity
