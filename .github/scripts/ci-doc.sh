. $(dirname "$0")/ci-common.sh

# Check cargo doc
cargo doc --features $non_exclusive_features --no-deps

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