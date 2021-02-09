cargo test

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo test --target i686-unknown-linux-gnu
fi

python examples/build.py

# Test with DummyVM (each test in a separate run)
cd vmbindings/dummyvm
for t in $(ls src/tests/ -I mod.rs | sed -n 's/\.rs$//p'); do
    cargo test -- $t;
done;
