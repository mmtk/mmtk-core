. $(dirname "$0")/ci-common.sh

for_all_features "cargo test"

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    # Skipping one test - see Issue #229. Remove the skip part when the bug is fixed.
    # After the bug is fixed, change the following line to 'for_all_features "cargo test --target i686-unknown-linux-gnu"'
    cargo test --target i686-unknown-linux-gnu -- --skip test_side_metadata_try_mmap_metadata_chunk
fi

python examples/build.py

# Test with DummyVM (each test in a separate run)
cd vmbindings/dummyvm
for t in $(ls src/tests/ -I mod.rs | sed -n 's/\.rs$//p'); do
    cargo test -- $t;
done;
