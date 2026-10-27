#!/bin/sh
# Build Pigeon.app and install it in /Applications (PER-4), quitting the
# installed copy first if it runs. The installed copy is the user's; run
# this again after each merge.
set -e
cd "$(dirname "$0")/.."
APP=$(scripts/bundle.sh)
DEST=/Applications/Pigeon.app

if pgrep -f "$DEST/Contents/MacOS/Pigeon" >/dev/null; then
  echo "Quitting the installed Pigeon…" >&2
  osascript -e "quit app \"$DEST\"" >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$DEST/Contents/MacOS/Pigeon" >/dev/null || break
    sleep 1
  done
  if pgrep -f "$DEST/Contents/MacOS/Pigeon" >/dev/null; then
    echo "error: the installed Pigeon is still running; quit it and try again" >&2
    exit 1
  fi
fi

rm -rf "$DEST"
ditto "$APP" "$DEST"
# So macOS offers it as a mail app and finds its icon at once.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST"
echo "Installed $DEST" >&2
