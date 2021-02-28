. $(dirname "$0")/ci-common.sh

# Check cargo doc
cargo doc --features $non_exclusive_features --no-deps

# Check tutorial code
tutorial_code_dir=$project_root/docs/tutorial/code/mygc_semispace
cp -r $tutorial_code_dir $project_root/src/plan/mygc
echo "pub mod mygc;" >> $project_root/src/plan/mod.rs
cargo build