set -xe

. $(dirname "$0")/ci-common.sh

export RUSTFLAGS="-D warnings"

# check plan
cargo clippy --features nogc
cargo clippy --features nogc_lock_free
cargo clippy --features nogc_no_zeroing
cargo clippy --features semispace
cargo clippy --features gencopy
# check features
cargo clippy --features nogc,sanity
cargo clippy --features semispace,sanity
cargo clippy --features gencopy,sanity
cargo clippy --features nogc,vm_space,code_space,ro_space
cargo clippy --features nogc,lockfreeimmortalspace
cargo clippy --features semispace,vm_space,code_space,ro_space
# check for tests
cargo clippy --tests --features nogc
# check for dummyvm
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features nogc
cargo clippy --manifest-path=vmbindings/dummyvm/Cargo.toml --features semispace

# check for different implementations of heap layout
cargo clippy --features nogc,force_32bit_heap_layout
# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo clippy --target x86_64-unknown-linux-gnu --features nogc
    cargo clippy --target x86_64-unknown-linux-gnu --features nogc,force_32bit_heap_layout
fi 

# check format
cargo fmt -- --check
