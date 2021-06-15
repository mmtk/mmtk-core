. $(dirname "$0")/ci-common.sh

# Check cargo doc
# We generate docs including private items so it would be easier for MMTk developers (GC implementers). However,
# this could be confusing to MMTk users (binding implementers), as they may find items in the doc which
# are not visible to a binding. If we exclude private items, the doc would be easier for the users, but would hide
# implementation details for developers.
cargo doc --features $non_exclusive_features --no-deps --document-private-items

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

cargo install mdbook
mdbook build $project_root/docs/portingguide
mdbook build $project_root/docs/tutorial