#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
VERSION=$(sh "$ROOT/packaging/version.sh")
case "${1:-public}" in
  public)
    OUTPUT="$ROOT/dist/ChickLink-Setup-v$VERSION.exe"
    makensis -V3 -DROOT="$ROOT" -DOUTPUT="$OUTPUT" -DVERSION="$VERSION" -DVI_VERSION="$VERSION.0" "$ROOT/packaging/windows/installer.nsi"
    ;;
  private)
    test -n "${2:-}" || { echo 'Supply the private Windows config directory.' >&2; exit 2; }
    CONFIG=$(CDPATH= cd -- "$2" && pwd)
    mkdir -p "$ROOT/dist/private-v$VERSION"
    OUTPUT="$ROOT/dist/private-v$VERSION/ChickLink-Setup-Private-v$VERSION.exe"
    makensis -V3 -DROOT="$ROOT" -DOUTPUT="$OUTPUT" -DVERSION="$VERSION" -DVI_VERSION="$VERSION.0" -DPRIVATE_CONFIG="$CONFIG" "$ROOT/packaging/windows/installer.nsi"
    ;;
  *) echo 'Usage: build-installer.sh public|private [private-config-directory]' >&2; exit 2 ;;
esac
shasum -a 256 "$OUTPUT" > "$OUTPUT.sha256"
