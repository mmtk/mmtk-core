. $(dirname "$0")/ci-common.sh

# rustdoc.yml will copy the docs from respective directories to a directory for publishing.
# If the output path is changed in this script, we need to update rustdoc.yml as well.

# deny warnings for rustdoc
export RUSTDOCFLAGS="-D warnings -D missing_docs"

# Check cargo doc
# We document public and private items for MMTk developers (GC implementers).
# Private items are annotated with padlock emojis by rustdoc
cargo doc --features $non_exclusive_features --no-deps --document-private-items

# Check tutorial code
tutorial_code_dir=$project_root/docs/userguide/src/tutorial/code/mygc_semispace
# Clear the dir and copy again
rm -rf $project_root/src/plan/mygc
cp -r $tutorial_code_dir $project_root/src/plan/mygc
# If we havent appended the mod line, append it
if ! cat $project_root/src/plan/mod.rs | grep -q "pub mod mygc;"; then
    echo "pub mod mygc;" >> $project_root/src/plan/mod.rs
fi
cargo build

# Check dummyvm in portingguide
cargo build --manifest-path $dummyvm_toml

# Install mdbook using the stable toolchain and the default target
unset CARGO_BUILD_TARGET

# mdbook-admonish does not support mdbook 0.5. So we pin the version to 0.4 for mdbook.
# When the issue (https://github.com/tommilligan/mdbook-admonish/issues/233) is resolved, we can upgrade mdbook to 0.5.
cargo +stable install mdbook --version "^0.4"
cargo +stable install mdbook-admonish --version "=1.20.0"
# It seems we don't need a specific version for mdbook-hide atm.
cargo +stable install mdbook-hide
rustup run stable mdbook build $project_root/docs/userguide
