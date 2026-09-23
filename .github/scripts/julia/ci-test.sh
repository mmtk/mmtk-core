set -xe

cur=$(realpath $(dirname "$0"))

# Build debug - skip this. Otherwise it takes too long
# cd $cur
# ./ci-build.sh debug

# Patch some tests to skip
. $(dirname "$0")/ci-test-patching.sh

# Build release
cd $cur
./ci-build.sh release Immix

export MMTK_PLAN=Immix

# Use release build to run tests
cd $cur
./ci-test-gc-core.sh
