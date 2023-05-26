. $(dirname "$0")/ci-common.sh

export RUST_BACKTRACE=1
# Run all tests with 1G heap
export MMTK_GC_TRIGGER=FixedHeapSize:1000000000

for_all_features "cargo test"

# target-specific features
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo test --features perf_counter
fi

./examples/build.py

ALL_PLANS=$(sed -n '/enum PlanSelector/,/}/p' src/util/options.rs | xargs | grep -o '{.*}' | grep -o '\w\+')

# Test with DummyVM (each test in a separate run)
cd vmbindings/dummyvm
for fn in $(ls src/tests/*.rs); do
    t=$(basename -s .rs $fn)

    if [[ $t == "mod" ]]; then
        continue
    fi

    # Get the required plans.
    # Some tests need to be run with multiple plans because
    # some bugs can only be reproduced in some plans but not others.
    PLANS=$(sed -n 's/^\/\/ *GITHUB-CI: *MMTK_PLAN=//p' $fn)
    if [[ $PLANS == 'all' ]]; then
        PLANS=$ALL_PLANS
    elif [[ -z $PLANS ]]; then
        PLANS=NoGC
    fi

    # Some tests need some features enabled.
    FEATURES=$(sed -n 's/^\/\/ *GITHUB-CI: *FEATURES=//p' $fn)

    # Run the test with each plan it needs.
    for MMTK_PLAN in $PLANS; do
        env MMTK_PLAN=$MMTK_PLAN cargo test --features "$FEATURES" -- $t;
    done
done

