set -e

# run tests that are more specific to the GC
# this list was built intuitively based on tests that 
# call the gc manually, have finalizers, have multiple threads, etc.

. $(dirname "$0")/common.sh

declare -a test_names=(
    "gc"                            # Julia's GC specific testset
    "threads"                       # Tests for multithreading (with some GC interaction)
    "cmdlineargs"                   # Has tests that spawn other processes
    "compiler"                      # Some tests for allocation, compiled code checks
    "misc"                          # set of miscelaneous tests, include finalizers, eg
    "core"                          # should have the core of the Julia code
    "dict"                          # tests for weak references
)

for i in "${test_names[@]}"
do
    # echo "Token: '$i'"
    test=`sed 's/\"\(.*\)\"/\1/' <<< $i`
    if [[ ! -z "$test" ]]; then
        echo $test

        echo "-> Run"
        ci_run_jl_test $test
    fi
done
