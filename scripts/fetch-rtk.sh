#!/usr/bin/env bash
# Fetches the pinned rtk binary for bundling. Idempotent.
set -euo pipefail
RTK_VERSION="${RTK_VERSION:-0.42.4}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
DEST="$ROOT/src-tauri/binaries/rtk-$TRIPLE"

if [ -x "$DEST" ] && "$DEST" --version 2>/dev/null | grep -q "$RTK_VERSION"; then
  echo "rtk $RTK_VERSION already staged at $DEST"
  exit 0
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
# --git + tag guarantees we build the rtk-ai project, not a name-squatted crate.
cargo install --git https://github.com/rtk-ai/rtk --tag "v$RTK_VERSION" --locked --root "$STAGE" rtk
mkdir -p "$ROOT/src-tauri/binaries"
cp "$STAGE/bin/rtk" "$DEST"
chmod 755 "$DEST"
echo "staged rtk $RTK_VERSION -> $DEST"
