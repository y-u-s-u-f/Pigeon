#!/bin/sh
# Build Pigeon in release mode, sign it with the stable "Pigeon Dev" identity
# (so the Keychain keeps trusting it across rebuilds), then run it.
# Honours CARGO_TARGET_DIR. Extra arguments are passed to the app.
set -e
cd "$(dirname "$0")/.."
cargo build --release
BIN="${CARGO_TARGET_DIR:-target}/release/pigeon"
if security find-identity -p codesigning | grep -q '"Pigeon Dev"'; then
  codesign -f -s "Pigeon Dev" --identifier com.pigeon.dev "$BIN" 2>/dev/null
else
  echo "warning: no \"Pigeon Dev\" signing identity; the Keychain will ask for access after each build" >&2
fi
exec "$BIN" "$@"
