#!/usr/bin/env bash
# sign-file.sh FILE KEY PASSWORD_FILE: writes FILE.sig, the launcher update
# signature of FILE (tauri signer, minisign format), with the release key.
#
# tauri-cli 1.6's signer panics on a file name without an extension
# (mezeporta-bin) and reads the key password only from its -p option, so this
# signs a copy named payload.bin (the signature covers the bytes, not the
# name) and passes the password from its file; it is on the command line only
# while the signer runs, on the build machine.
set -euo pipefail
FILE="$(realpath "$1")" KEY="$2" PASSWORD_FILE="$3"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d "$(dirname "$FILE")/.sign.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
cp "$FILE" "$TMP/payload.bin"
(cd "$REPO" && npx tauri signer sign -f "$KEY" -p "$(cat "$PASSWORD_FILE")" "$TMP/payload.bin" > /dev/null 2> "$TMP/err") || true
if [[ ! -s "$TMP/payload.bin.sig" ]]; then
  echo "signing $FILE failed:" >&2
  grep -v -i 'private' "$TMP/err" | head -n 5 >&2
  exit 1
fi
mv "$TMP/payload.bin.sig" "$FILE.sig"
