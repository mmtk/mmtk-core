set -xe

. $(dirname "$0")/common.sh

rustup toolchain install $RUSTUP_TOOLCHAIN
rustup component add clippy --toolchain $RUSTUP_TOOLCHAIN
rustup component add rustfmt --toolchain $RUSTUP_TOOLCHAIN
rustup override set $RUSTUP_TOOLCHAIN
sudo sysctl kernel.perf_event_paranoid=3

"$MMTK_CORE_PATH/.github/scripts/ci-replace-mmtk-dep.sh" \
    "$MMTK_JULIA_DIR/Cargo.toml" \
    --mmtk-core-path "$MMTK_CORE_PATH"
