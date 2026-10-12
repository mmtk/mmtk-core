#!/bin/sh
# Regenerate src/policy/spanspace/sizeclasses.rs from Go's size-class generator.
#
# Usage: tools/spanspace-sizeclasses/gen.sh <path to a golang/go checkout> [go binary]
#
# The Go checkout supplies src/runtime/_mkmalloc/{mksizeclasses.go,constants.go}
# and LICENSE. They are compiled, unmodified, together with emit_rust.go, which
# only changes the output format to Rust. No module dependencies are needed, and
# any Go toolchain >= 1.21 works.
#
# Set GO_REF to the Go commit the checkout is at (default: the reference commit
# of the ConcurrentSpan port). Set GO_LICENSE to Go's LICENSE file if the
# checkout does not have one (e.g. a sparse checkout of src/ only); the LICENSE
# shipped with any official Go toolchain has the same text.
set -eu
GO_SRC=${1:?usage: gen.sh <go checkout> [go binary]}
GO=${2:-go}
GO_REF=${GO_REF:-"golang/go 2c3c1caad2"}
GO_LICENSE=${GO_LICENSE:-"$GO_SRC/LICENSE"}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp "$GO_SRC/src/runtime/_mkmalloc/mksizeclasses.go" "$WORK/"
cp "$GO_SRC/src/runtime/_mkmalloc/constants.go" "$WORK/"
cp "$HERE/emit_rust.go" "$WORK/"
# Embed Go's LICENSE text so the generated file carries the full notice.
{
  printf 'package main\n\nvar goLicense = []byte(`'
  sed "s/\`/'/g" "$GO_LICENSE"
  printf '`)\n'
} > "$WORK/license.go"
printf 'module sizeclassgen\n\ngo 1.21\n' > "$WORK/go.mod"

(cd "$WORK" && GO_REF="$GO_REF" GOTOOLCHAIN=local "$GO" run .) > "$WORK/sizeclasses.rs"
cp "$WORK/sizeclasses.rs" "$REPO/src/policy/spanspace/sizeclasses.rs"
echo "Wrote $REPO/src/policy/spanspace/sizeclasses.rs"
