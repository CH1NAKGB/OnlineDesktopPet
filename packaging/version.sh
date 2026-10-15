#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VERSION=$(awk '
  /^\[workspace.package\]$/ { in_package=1; next }
  /^\[/ { in_package=0 }
  in_package && /^version = "/ { gsub(/"/, "", $3); print $3; exit }
' "$ROOT/Cargo.toml")
case "$VERSION" in
  *[!0-9.]*|.*|*..*|*.) echo 'Cargo.toml workspace version must be X.Y.Z' >&2; exit 1 ;;
esac
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || {
  echo 'Cargo.toml workspace version must be X.Y.Z' >&2; exit 1;
}
printf '%s\n' "$VERSION"
