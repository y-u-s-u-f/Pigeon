#!/bin/sh
# Build Pigeon.app in release mode (scripts/bundle.sh: signed with the stable
# "Pigeon Dev" identity, so the Keychain keeps trusting it across rebuilds)
# and run it from the bundle, so notifications work in dev too. The binary
# runs directly, so the environment (PIGEON_DEMO, PIGEON_DATA_DIR,
# PIGEON_DEV, …) passes through and its output stays in this terminal.
# Honours CARGO_TARGET_DIR. Extra arguments are passed to the app.
set -e
cd "$(dirname "$0")/.."
APP=$(scripts/bundle.sh)
exec "$APP/Contents/MacOS/Pigeon" "$@"
