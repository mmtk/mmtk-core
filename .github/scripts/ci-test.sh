. $(dirname "$0")/ci-common.sh

for_all_features "cargo test"

# For x86_64-linux, also check for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    for_all_features "cargo test --target i686-unknown-linux-gnu"
    cargo test --features perf_counter
fi

python examples/build.py

# Test with DummyVM (each test in a separate run)
cd vmbindings/dummyvm
for t in $(ls src/tests/ -I mod.rs | sed -n 's/\.rs$//p'); do
    MMTK_PLAN=$p cargo test -- $t;
done;
