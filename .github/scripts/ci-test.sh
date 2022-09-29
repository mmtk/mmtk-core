. $(dirname "$0")/ci-common.sh

export RUST_BACKTRACE=1

for_all_features "cargo test"

# target-specific features
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo test --features perf_counter
fi

python examples/build.py

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
    PLANS=$(sed -n 's;^//\s*GITHUB-CI:\s*MMTK_PLAN=\(\w\+\)\s*$;\1;p' $fn)
    if [[ $PLANS == 'all' ]]; then
        PLANS=$ALL_PLANS
    elif [[ -z $PLANS ]]; then
        PLANS=NoGC
    fi

    # Some tests need some features enabled.
    FEATURES=$(sed -n 's;^//\s*GITHUB-CI:\s*FEATURES=\([a-zA-Z0-9_,]\+\)\s*$;\1;p' $fn)

    # Run the test with each plan it needs.
    for MMTK_PLAN in $PLANS; do
        env MMTK_PLAN=$MMTK_PLAN cargo test --features "$FEATURES" -- $t;
    done
done

