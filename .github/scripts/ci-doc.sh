set -xe

cargo doc --features $non_exclusive_features --no-deps
