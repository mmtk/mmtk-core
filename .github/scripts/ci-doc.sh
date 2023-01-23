. $(dirname "$0")/ci-common.sh

# rustdoc.yml will copy the docs from respective directories to a directory for publishing.
# If the output path is changed in this script, we need to update rustdoc.yml as well.

# deny warnings for rustdoc
export RUSTFLAGS="-D warnings"

# Check cargo doc
# We generate two versions of docs: one with only public items for binding developers for our API, and
# the other with both public and private items for MMTk developers (GC implementers).
cargo doc --features $non_exclusive_features --no-deps --target-dir target/mmtk-public
cargo doc --features $non_exclusive_features --no-deps --document-private-items --target-dir target/mmtk-full

# Check tutorial code
tutorial_code_dir=$project_root/docs/tutorial/code/mygc_semispace
# Clear the dir and copy again
rm -rf $project_root/src/plan/mygc
cp -r $tutorial_code_dir $project_root/src/plan/mygc
# If we havent appended the mod line, append it
if ! cat $project_root/src/plan/mod.rs | grep -q "pub mod mygc;"; then
    echo "pub mod mygc;" >> $project_root/src/plan/mod.rs
fi
cargo build

# Install mdbook using the stable toolchain (mdbook uses scoped-tls which requires rust 1.59.0)
cargo +stable install mdbook
mdbook build $project_root/docs/portingguide
mdbook build $project_root/docs/tutorial