#!/bin/sh
# Build Pigeon.app (PER-4) in release mode: Info.plist from packaging/
# (bundle id com.pigeon.dev, mailto: and pigeon: handler), the bird icon and
# menu bar mark from packaging/make_icon.py (MA-056), signed
# with the stable "Pigeon Dev" identity so the Keychain, notification
# permission and the default-mail-app choice stay tied to one identity.
# Honours CARGO_TARGET_DIR. Prints the bundle's path.
set -e
cd "$(dirname "$0")/.."
cargo build --release >&2
TARGET="${CARGO_TARGET_DIR:-target}"
APP="$TARGET/release/Pigeon.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

ICONS="$TARGET/icon"
if [ ! -f "$ICONS/Pigeon.icns" ] || [ packaging/make_icon.py -nt "$ICONS/Pigeon.icns" ]; then
  python3 packaging/make_icon.py "$ICONS" >&2
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET/release/pigeon" "$APP/Contents/MacOS/Pigeon"
sed "s/@VERSION@/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
cp "$ICONS/Pigeon.icns" "$ICONS/AppIcon.png" "$ICONS/MenuBarIcon.png" "$ICONS/MenuBarIcon@2x.png" \
  "$APP/Contents/Resources/"

if security find-identity -p codesigning | grep -q '"Pigeon Dev"'; then
  codesign -f -s "Pigeon Dev" --identifier com.pigeon.dev "$APP" 2>/dev/null
else
  echo "warning: no \"Pigeon Dev\" signing identity; signing ad hoc, so the Keychain will ask for access after each build" >&2
  codesign -f -s - --identifier com.pigeon.dev "$APP" 2>/dev/null
fi
echo "$APP"
