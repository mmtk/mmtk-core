set -xe

cargo doc --features semispace --no-deps -Z crate-versions
