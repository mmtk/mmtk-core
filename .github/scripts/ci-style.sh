set -xe

export RUSTFLAGS="-D warnings"
targets=("x86_64-unknown-linux-gnu" "i686-unknown-linux-gnu" "x86_64-apple-darwin")

for t in "${targets[@]}"
do
# check plan
cargo clippy --target $t --features nogc
cargo clippy --target $t --features nogc_lock_free
cargo clippy --target $t --features nogc_no_zeroing
cargo clippy --target $t --features semispace
# check features
cargo clippy --target $t --features nogc,sanity
cargo clippy --target $t --features semispace,sanity
cargo clippy --target $t --features nogc,vm_space,code_space,ro_space
cargo clippy --target $t --features nogc,lockfreeimmortalspace
cargo clippy --target $t --features semispace,vm_space,code_space,ro_space
# check for tests
cargo clippy --target $t --tests --features nogc
# check for dummyvm
cargo clippy --target $t --manifest-path=vmbindings/dummyvm/Cargo.toml --features nogc
cargo clippy --target $t --manifest-path=vmbindings/dummyvm/Cargo.toml --features semispace
# check for different implementations of heap layout
cargo clippy --target $t --features nogc
cargo clippy --target $t --features nogc,force_32bit_heap_layout
done

# check format
cargo fmt -- --check
