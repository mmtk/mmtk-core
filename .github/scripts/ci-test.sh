set -xe

cargo test --features nogc
cargo test --features semispace
python examples/build.py

cd vmbindings/dummyvm
cargo test fixed_live --features nogc
cargo test gcbench --features nogc