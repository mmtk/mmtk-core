set -xe

# Execute this script under the root folder of this repo. Otherwise it will fail.

# Build plans
cargo build --features nogc
cargo build --features semispace

# Build features
cargo build --features nogc,vm_space
cargo build --features nogc,vm_space,code_space,ro_space
cargo build --features semispace,vm_space
cargo build --features semispace,vm_space,code_space,ro_space
cargo build --features nogc,sanity
cargo build --features semispace,sanity
