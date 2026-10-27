#!/bin/sh
# Build Pigeon.app (PER-4) in release mode: Info.plist from packaging/
# (bundle id com.pigeon.dev, mailto: handler), the placeholder icon, signed
# with the stable "Pigeon Dev" identity so the Keychain, notification
# permission and the default-mail-app choice stay tied to one identity.
# Honours CARGO_TARGET_DIR. Prints the bundle's path.
set -e
cd "$(dirname "$0")/.."
cargo build --release >&2
TARGET="${CARGO_TARGET_DIR:-target}"
APP="$TARGET/release/Pigeon.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

ICNS="$TARGET/Pigeon.icns"
if [ ! -f "$ICNS" ] || [ packaging/make_icon.py -nt "$ICNS" ]; then
  python3 packaging/make_icon.py "$ICNS" >&2
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET/release/pigeon" "$APP/Contents/MacOS/Pigeon"
sed "s/@VERSION@/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
cp "$ICNS" "$APP/Contents/Resources/Pigeon.icns"

if security find-identity -p codesigning | grep -q '"Pigeon Dev"'; then
  codesign -f -s "Pigeon Dev" --identifier com.pigeon.dev "$APP" 2>/dev/null
else
  echo "warning: no \"Pigeon Dev\" signing identity; signing ad hoc, so the Keychain will ask for access after each build" >&2
  codesign -f -s - --identifier com.pigeon.dev "$APP" 2>/dev/null
fi
echo "$APP"
