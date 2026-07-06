SCRIPT_DIR=$(realpath "$(dirname "$0")")
export MMTK_CORE_PATH=${MMTK_CORE_PATH:-$(realpath -m "$SCRIPT_DIR/../../..")}
export JULIA_PATH=${JULIA_PATH:-$(realpath -m "$MMTK_CORE_PATH/../julia")}
export MMTK_JULIA_DIR=$JULIA_PATH/src/gc-mmtk/mmtk_julia

if [ -f "$MMTK_JULIA_DIR/rust-toolchain" ]; then
    RUSTUP_TOOLCHAIN=$(cat "$MMTK_JULIA_DIR/rust-toolchain")
else
    RUSTUP_TOOLCHAIN=$(cat "$MMTK_CORE_PATH/rust-toolchain")
fi
JULIA_TEST_ARGS='--check-bounds=yes --startup-file=no --depwarn=error'

# Make sure we have enough heap to build Julia
export MMTK_MIN_HSIZE_G=0.5
export MMTK_MAX_HSIZE_G=16
# Make sure we do not get OOM killed.
total_mem=$(free -m | awk '/^Mem:/ {print $2}')
export JULIA_TEST_MAXRSS_MB=$total_mem

ci_run_jl_test() {
    test=$1
    threads=$2

    # if no argument is given, use 2 as default
    if [ -z "$threads" ]; then
        threads=2
    fi

    cd $JULIA_PATH
    export JULIA_CPU_THREADS=$threads

    # Directly run runtests.jl: There could be some issues with some test suites. We better just use their build script.
    # $JULIA_PATH/julia $JULIA_TEST_ARGS $JULIA_PATH/test/runtests.jl --exit-on-error $test

    # Run with their build script
    make test-$test
}
