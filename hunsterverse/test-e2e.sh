#!/usr/bin/env bash
# test-e2e.sh: the real launcher (Linux build) end to end on mhfz-dev, with
# local stand-ins for the game server, the account API and the GitHub release
# (e2e/fake_servers.py). Drives the window through tauri-driver and
# WebKitWebDriver (e2e/drive.py) under Xvfb. No real server, account or game
# file is used.
#
#   hunsterverse/test-e2e.sh NEW_BIN NEXT_BIN
#
# NEW_BIN:  this version's Linux launcher (e.g. 1.5.4-hv2, built by build-release.sh or
#           `npx tauri build -b none --target x86_64-unknown-linux-gnu --config src-tauri/tauri.linux.conf.json`)
# NEXT_BIN: the same source built with a higher version (e.g. 1.5.4-hv3), the "new release"
#           for the update test. It is signed here with the real release key
#           (TAURI_PRIVATE_KEY_PATH / HV_KEY_PASSWORD_FILE, as build-release.sh),
#           since NEW_BIN only accepts that key.
#
# Needs: Xvfb, WebKitWebDriver (apt webkit2gtk-driver), tauri-driver
# (cargo install tauri-driver --locked), python3, npx (tauri signer).
# Checks:
#   patch:  first login installs the stable patch; the Patch panel shows
#           installed/served versions; choosing the pre-release asks for the
#           API's confirmation, then the launcher logs in again and installs
#           it; choosing Stable puts the stable files back and removes the
#           added file; the account API holds each choice.
#   update: a tampered download is refused with the file unchanged; the
#           signed one replaces the binary and the launcher restarts into it
#           in place (same PID).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
NEW_BIN="$(realpath "$1")"
NEXT_BIN="$(realpath "$2")"
KEY="${TAURI_PRIVATE_KEY_PATH:-$HOME/.secrets/mezeporta-updater.key}"
KEY_PASSWORD_FILE="${HV_KEY_PASSWORD_FILE:-$HOME/.secrets/mezeporta-updater.key.password}"
GP=${GP:-18480} AP=${AP:-18481} RP=${RP:-18482} DISPLAY_NUM=${DISPLAY_NUM:-97}
export PATH="$HOME/.cargo/bin:$PATH"

for t in Xvfb WebKitWebDriver tauri-driver python3 npx; do
  command -v "$t" >/dev/null || { echo "$t is missing" >&2; exit 1; }
done

WORK="$(mktemp -d /root/work/mezeporta-e2e.XXXXXX)"
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  # A launcher restarted by exec keeps its PID under WebKitWebDriver; stop any left.
  for p in $(python3 -I - "$WORK/game/mezeporta-bin" <<'EOF'
import os, sys
for pid in filter(str.isdigit, os.listdir("/proc")):
    try:
        if os.readlink("/proc/%s/exe" % pid).replace(" (deleted)", "") == sys.argv[1]:
            print(pid)
    except OSError:
        pass
EOF
  ); do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  if [[ -z "${KEEP_WORK:-}" ]]; then rm -rf "$WORK"; else echo "kept $WORK"; fi
}
trap cleanup EXIT

STATE="$WORK/state" GAME="$WORK/game"
mkdir -p "$STATE/stable/dat" "$STATE/slots/mods/dat" "$STATE/release" "$GAME/dat" "$GAME/Mezeporta"

# Patches: stable v1.30 and pre-release "mods" v1.31-pre2 (made-up bytes).
printf 'stable v1.30 mhfo-hd\n' > "$STATE/stable/mhfo-hd.dll"
printf 'stable a.pac\n' > "$STATE/stable/dat/a.pac"
printf 'pre mods mhfo-hd\n' > "$STATE/slots/mods/mhfo-hd.dll"
printf 'stable a.pac\n' > "$STATE/slots/mods/dat/a.pac"
printf 'pre mods loader\n' > "$STATE/slots/mods/hvmods.dll"
echo v1.30 > "$STATE/stable.version"
echo v1.31-pre2 > "$STATE/slots/mods.version"
echo stable > "$STATE/channel"
echo no > "$STATE/eligible"

# Game folder: the launcher next to "original" game files.
cp "$NEW_BIN" "$GAME/mezeporta-bin"
printf 'original mhfo-hd\n' > "$GAME/mhfo-hd.dll"
printf 'original a.pac\n' > "$GAME/dat/a.pac"
echo keep > "$GAME/other.txt"
python3 -I - "$GAME/Mezeporta/config.json" "$GP" <<'EOF'
import json, sys
ep = {"url": "127.0.0.1", "name": "Hunsterverse", "launcherPort": int(sys.argv[2]), "gamePort": 53310,
      "version": "ZZ", "isRemote": False, "serverMode": "api", "gameFolder": None}
cfg = {"endpoints": [ep], "current_endpoint": dict(ep),
       "launcher_prefs": {"preloadControllerDlls": False, "friendSignature": "none",
                          "winePrefixMode": "system", "winePrefixCustomPath": None, "serverMode": "api"}}
json.dump(cfg, open(sys.argv[1], "w"), indent=2)
EOF

python3 -I "$HERE/e2e/fake_servers.py" "$STATE" "$GP" "$AP" "$RP" > "$WORK/servers.out" 2>&1 &
PIDS+=($!)
Xvfb ":$DISPLAY_NUM" -screen 0 1280x800x24 -nolisten tcp > "$WORK/xvfb.out" 2>&1 &
PIDS+=($!)
for _ in $(seq 50); do grep -q listening "$WORK/servers.out" 2>/dev/null && break; sleep 0.1; done
grep -q listening "$WORK/servers.out" || { cat "$WORK/servers.out" >&2; echo "fake servers did not start" >&2; exit 1; }

(
  cd "$GAME"
  export DISPLAY=":$DISPLAY_NUM" GDK_BACKEND=x11 WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 RUST_BACKTRACE=1
  export MEZEPORTA_UPDATE_URL="http://127.0.0.1:$RP/latest.json"
  export MEZEPORTA_HV_ACCOUNT_API="http://127.0.0.1:$AP"
  exec tauri-driver --port 4444
) > "$WORK/driver.out" 2>&1 &
PIDS+=($!)
sleep 2

echo "== patch choice"
python3 -I "$HERE/e2e/drive.py" patch "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
grep -q " PUT /api/account/patch-channel 409" "$STATE/requests.log" || { echo "FAIL no 409 confirmation round" >&2; exit 1; }
[[ $(grep -c " PUT /api/account/patch-channel 200" "$STATE/requests.log") == 2 ]] || { echo "FAIL expected 2 saved choices" >&2; exit 1; }
grep -q " GET /prerelease/mods/hvmods.dll 200" "$STATE/requests.log" || { echo "FAIL the pre-release file was not downloaded from its slot" >&2; exit 1; }
echo "ok   API log: 409 confirmation, 2 saved choices; pre-release files came from /prerelease/mods"

echo "== launcher self-update"
cp "$NEXT_BIN" "$STATE/release/mezeporta-bin"
"$HERE/sign-file.sh" "$STATE/release/mezeporta-bin" "$KEY" "$KEY_PASSWORD_FILE"
printf 'Test release for the e2e test.\n' > "$WORK/notes.txt"
python3 -I "$HERE/make_latest_json.py" 1.5.4-hv3 "$WORK/notes.txt" "http://127.0.0.1:$RP" "$STATE/release" > "$STATE/release/latest.json"
python3 -I "$HERE/e2e/drive.py" update "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
[[ $(grep -c "^$RP GET /latest.json 200" "$STATE/requests.log") -ge 2 ]] || { echo "FAIL the restarted launcher did not check for updates again" >&2; exit 1; }
echo "ok   the restarted launcher checked latest.json again (now up to date)"
echo "all e2e checks passed"
