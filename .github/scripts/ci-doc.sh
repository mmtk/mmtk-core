set -xe

. $(dirname "$0")/ci-common.sh

cargo doc --features $non_exclusive_features --no-deps
