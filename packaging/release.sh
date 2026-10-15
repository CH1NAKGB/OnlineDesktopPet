#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(sh packaging/version.sh)
RELEASE_NOTES="packaging/RELEASE-$VERSION.md"
[ -f "$RELEASE_NOTES" ] || { echo "Missing $RELEASE_NOTES" >&2; exit 1; }
STAGE=$(mktemp -d /tmp/chicklink-release.XXXXXX)
echo "Release staging: $STAGE"
export COPYFILE_DISABLE=1

cargo build --locked --release -p pet-client --target aarch64-apple-darwin
cargo build --locked --release -p pet-client --target x86_64-apple-darwin
cargo build --locked --release -p pet-client --target x86_64-pc-windows-gnu

APP="$STAGE/macos/Chick Link.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/font-licenses" "$STAGE/windows/font-licenses" dist
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
lipo -create target/aarch64-apple-darwin/release/pet-client \
    target/x86_64-apple-darwin/release/pet-client -output "$APP/Contents/MacOS/Chick Link"
cp pet-client/assets/fonts/LICENSE.txt pet-client/assets/fonts/AUTHORS.txt \
    pet-client/assets/fonts/README.md "$APP/Contents/Resources/font-licenses/"
cp "$RELEASE_NOTES" "$STAGE/macos/"
ln -s /Applications "$STAGE/macos/Applications"
xattr -cr "$APP"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
hdiutil create -volname "Chick Link $VERSION" -srcfolder "$STAGE/macos" \
    -ov -format UDZO "dist/ChickLink-macos-universal-v$VERSION.dmg"

cp target/x86_64-pc-windows-gnu/release/pet-client.exe "$STAGE/windows/"
cp "$RELEASE_NOTES" "$STAGE/windows/"
cp pet-client/assets/fonts/LICENSE.txt pet-client/assets/fonts/AUTHORS.txt \
    pet-client/assets/fonts/README.md "$STAGE/windows/font-licenses/"
(cd "$STAGE/windows"; zip -qr "$STAGE/windows.zip" .)
cp "$STAGE/windows.zip" "dist/ChickLink-windows-x64-v$VERSION.zip"
cp target/x86_64-pc-windows-gnu/release/pet-client.exe "dist/ChickLink-windows-x64-v$VERSION.exe"
python3 packaging/build-server-source.py "$STAGE/server.zip"
cp "$STAGE/server.zip" "dist/ChickLink-server-docker-source-v$VERSION.zip"
shasum -a 256 "dist/ChickLink-macos-universal-v$VERSION.dmg" \
    "dist/ChickLink-windows-x64-v$VERSION.exe" \
    "dist/ChickLink-windows-x64-v$VERSION.zip" \
    "dist/ChickLink-server-docker-source-v$VERSION.zip" > "dist/SHA256SUMS-v$VERSION.txt"
python3 packaging/check-release.py
echo "Release assets ready in dist/"
