set -xe

cargo test --features nogc
cargo test --features semispace

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo test --features nogc --target i686-unknown-linux-gnu
    cargo test --features semispace --target i686-unknown-linux-gnu
fi

python examples/build.py

# Test with DummyVM (each test in a separate run)
cd vmbindings/dummyvm
for p in $(find ../../src/plan -mindepth 1 -type d | xargs -L 1 basename); do
    for t in $(ls src/tests/ -I mod.rs | sed -n 's/\.rs$//p'); do
        cargo test --features $p -- $t;
    done;
done;
