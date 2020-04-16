set -xe

cargo test --features nogc
cargo test --features semispace
python examples/build.py