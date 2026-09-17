. $(dirname "$0")/ci-common.sh

# We may consider adding 'cargo fix' and 'cargo clippy --fix' here.
# However, fixing warnings and style issues may need human intervention
# for the best results. Unless we are confident that 'cargo fix' and 'cargo clippy --fix'
# produces good results, we should not add them to this CI pipeline.

# Currently we only run 'cargo fmt'.

# --- Format everything ---

cargo fmt
cargo fmt --manifest-path=macros/Cargo.toml
cargo fmt --manifest-path=docs/dummyvm/Cargo.toml
