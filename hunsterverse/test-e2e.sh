#!/usr/bin/env bash
# test-e2e.sh: the real launcher (Linux build) end to end on mhfz-dev, with
# local stand-ins for the game server, the account API and the GitHub release
# (e2e/fake_servers.py). Drives the window through tauri-driver and
# WebKitWebDriver (e2e/drive.py) under Xvfb. No real server, account or game
# file is used.
#
#   hunsterverse/test-e2e.sh NEW_BIN NEXT_BIN
#
# NEW_BIN:  this version's Linux launcher (e.g. 1.5.4-hv4, built by build-release.sh or
#           `npx tauri build -b none --target x86_64-unknown-linux-gnu --config src-tauri/tauri.linux.conf.json`)
# NEXT_BIN: the same source built with a higher version (e.g. 1.5.4-hv7), the "new release"
#           for the update test. It is signed here with the real release key
#           (TAURI_PRIVATE_KEY_PATH / HV_KEY_PASSWORD_FILE, as build-release.sh),
#           since NEW_BIN only accepts that key.
# Env: NEXT_VERSION  NEXT_BIN's version (default 1.5.4-hv7);
#      NEW_VERSION   NEW_BIN's version: if set, the header must show it;
#      SCENARIOS     default "patch events proton cancel report update" (e.g. "update" for an old NEW_BIN);
#      SHOTS         folder for screenshots of the character page;
#      STYLE         launcher look: 1 classic (default), 3 PS4.
#
# Needs: Xvfb, WebKitWebDriver (apt webkit2gtk-driver), tauri-driver
# (cargo install tauri-driver --locked), python3, npx (tauri signer).
# Checks:
#   patch:  first login installs the stable patch; the Patch panel shows
#           installed/served versions and, with a pre-release out, the link to
#           the full pre-release notes on the website (screenshot of the open
#           panel with SHOTS; clicking it opens that URL through a stand-in
#           xdg-open, no real browser); choosing the pre-release asks for the
#           API's confirmation, then the launcher logs in again and installs
#           it; choosing Stable puts the stable files back and removes the
#           added file; the account API holds each choice. Then the
#           pre-release again (hv4 patch cache): switching to Stable and back
#           downloads no file (counted in the fake server's request log), the
#           game folder checked after each switch.
#   events: the event badges come from the account API's events answers
#           (e2e/events-current.json, festa.json; made-up quests), and from the
#           game server's flags while that API is down. Screenshots go to
#           SHOTS_DIR, else SHOTS (default: $WORK/shots; set KEEP_WORK, SHOTS_DIR
#           or SHOTS to keep them).
#   proton: Steam Deck. No Wine (MEZEPORTA_WINE_CMD points nowhere) and a
#           stand-in Proton as the installer's MEZEPORTA_PROTON_CMD: Start runs
#           it even with the setting reset to "portable" (older launchers did
#           that), as "getcompatpath <game folder>" with an empty stdin, then
#           "runinprefix <meze-deps.exe> --stdin-b64" with the game settings on
#           stdin (hv7; "run" lost stdin in Proton's steam.exe); "proton"
#           stays set with Dev Mode off when Settings opens.
#   cancel: (hv4 fix) choose the pre-release, Cancel the update dialog and
#           start the game (stand-in Proton) on the stable files; open the
#           launcher again, Install, Cancel while the last file downloads (the
#           fake server stalls it half-way): no file of that patch reaches the
#           game folder, which holds the original files; the next login offers
#           the patch again and installs all of it.
#   report: (hv6) after the game starts above: game.log holds the stand-in
#           Proton's output and exit code; the stand-in "game" ended at once,
#           so the launcher opens with "The game closed right after it
#           started" (exit code 0): its Save report writes a report, "Debug
#           the next game start" is taken, Close marks it seen; Settings,
#           Advanced, Save report writes Mezeporta/logs/problem-report-*.txt
#           with the system, the settings, the last game start, the game
#           files and the logs (the game start included) and opens the log
#           folder; the next game start gets WINEDEBUG and (hv7: the stand-in
#           passes the environment on) PROTON_LOG=1, PROTON_LOG_DIR = the log
#           folder, SteamGameId=hunsterverse, and game.log says "debug launch";
#           at most 5 launcher log files; the password in none.
#   update: a tampered download is refused with the file unchanged; the
#           signed one replaces the binary and the launcher restarts into it
#           in place (same PID); with NEW_VERSION hv8 or later, the new
#           mezeporta.log says "started after an update from NEW_VERSION".
#   layout: the Patch button is part of the launcher layout and covers
#           nothing, at the default window size and a small one.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
NEW_BIN="$(realpath "$1")"
NEXT_BIN="$(realpath "$2")"
KEY="${TAURI_PRIVATE_KEY_PATH:-$HOME/.secrets/mezeporta-updater.key}"
KEY_PASSWORD_FILE="${HV_KEY_PASSWORD_FILE:-$HOME/.secrets/mezeporta-updater.key.password}"
NEXT_VERSION="${NEXT_VERSION:-1.5.4-hv7}" SCENARIOS="${SCENARIOS:-patch events proton cancel report update}"
export NEXT_VERSION
[[ -z "${SHOTS:-}" ]] || { mkdir -p "$SHOTS"; export SHOTS="$(realpath "$SHOTS")"; }
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
# The game helper the launcher hands to Wine/Proton (made-up bytes; the release
# zip has the real one in the same place).
mkdir -p "$GAME/Mezeporta/bin"
printf 'stand-in meze-deps\n' > "$GAME/Mezeporta/bin/meze-deps.exe"
python3 -I - "$GAME/Mezeporta/config.json" "$GP" "${STYLE:-1}" <<'EOF'
import json, sys
ep = {"url": "127.0.0.1", "name": "Hunsterverse", "launcherPort": int(sys.argv[2]), "gamePort": 53310,
      "version": "ZZ", "isRemote": False, "serverMode": "api", "gameFolder": None}
cfg = {"endpoints": [ep], "current_endpoint": dict(ep),
       "launcher_prefs": {"preloadControllerDlls": False, "friendSignature": "none",
                          "winePrefixMode": "system", "winePrefixCustomPath": None, "serverMode": "api"},
       "style": int(sys.argv[3])}
json.dump(cfg, open(sys.argv[1], "w"), indent=2)
EOF

# Stand-in Proton (as hvmhfz_ops/player-installer/tests): records each call,
# with the game settings the launcher writes to its stdin.
mkdir -p "$WORK/proton" "$STATE/compat"
{ echo '#!/bin/sh'
  echo "log='$STATE/proton-calls.log'"
  cat <<'EOF'
# hv7: "getcompatpath <folder>" (prefix set-up, stdin must stay empty) then
# "runinprefix ... --stdin-b64" (the game); END after the game's call only.
{ echo "ARGS $*"; echo "COMPAT ${STEAM_COMPAT_DATA_PATH:-}"; echo "WINEDEBUG ${WINEDEBUG:-}"
  echo "PROTONLOG ${PROTON_LOG:-}|${PROTON_LOG_DIR:-}|${SteamGameId:-}"
  case " $* " in
    *" --stdin-b64 "*) echo "STDIN $(cat)"; echo END ;;
    *) echo "PREP-STDIN-BYTES $(cat | wc -c)" ;;
  esac; } >> "$log"
[ "$1" = getcompatpath ] && echo 'Z:\game'
exit 0
EOF
} > "$WORK/proton/proton"
chmod +x "$WORK/proton/proton"

# Stand-in browser: the launcher opens links with xdg-open (Tauri's shell open,
# the `open` crate tries it first). It only records the URL, so no real browser
# starts during the test.
mkdir -p "$WORK/browser"
printf '#!/bin/sh\necho "$1" >> %q\n' "$STATE/browser-calls.log" > "$WORK/browser/xdg-open"
chmod +x "$WORK/browser/xdg-open"

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
  # Steam Deck set-up as install-hunsterverse.sh writes it, and no Wine.
  export MEZEPORTA_PROTON_CMD="$WORK/proton/proton" STEAM_COMPAT_DATA_PATH="$STATE/compat"
  export MEZEPORTA_WINE_CMD="$WORK/no-wine/wine"
  export PATH="$WORK/browser:$PATH"
  exec tauri-driver --port 4444
) > "$WORK/driver.out" 2>&1 &
PIDS+=($!)
sleep 2

if [[ " $SCENARIOS " == *" patch "* ]]; then
echo "== patch choice"
python3 -I "$HERE/e2e/drive.py" patch "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
grep -q " PUT /api/account/patch-channel 409" "$STATE/requests.log" || { echo "FAIL no 409 confirmation round" >&2; exit 1; }
[[ $(grep -c " PUT /api/account/patch-channel 200" "$STATE/requests.log") == 3 ]] || { echo "FAIL expected 3 saved choices" >&2; exit 1; }
grep -q " GET /prerelease/mods/hvmods.dll 200" "$STATE/requests.log" || { echo "FAIL the pre-release file was not downloaded from its slot" >&2; exit 1; }
echo "ok   API log: 409 confirmation, 3 saved choices; pre-release files came from /prerelease/mods"
fi

if [[ " $SCENARIOS " == *" events "* ]]; then
echo "== event badges"
cp "$HERE/e2e/events-current.json" "$HERE/e2e/festa.json" "$STATE/"
python3 -I "$HERE/e2e/drive.py" events "$GAME" "$STATE" "${SHOTS_DIR:-${SHOTS:-$WORK/shots}}" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
grep -q " GET /api/public/events/current 200" "$STATE/requests.log" || { echo "FAIL the launcher did not read the events API" >&2; exit 1; }
grep -q " GET /api/public/events/current 503" "$STATE/requests.log" || { echo "FAIL the fallback run did not try the events API" >&2; exit 1; }
echo "ok   API log: events read, and tried while down"
fi

if [[ " $SCENARIOS " == *" proton "* ]]; then
echo "== Proton (Steam Deck)"
python3 -I "$HERE/e2e/drive.py" proton "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
fi

if [[ " $SCENARIOS " == *" cancel "* ]]; then
echo "== cancelled patch"
python3 -I "$HERE/e2e/drive.py" cancel "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
grep -q " GET /prerelease/mods/dat/zz-cancel-test.pac stalled" "$STATE/requests.log" || { echo "FAIL the stalled download did not happen" >&2; exit 1; }
grep -q " GET /prerelease/mods/dat/zz-cancel-test.pac 200" "$STATE/requests.log" || { echo "FAIL the file was not downloaded again after the cancel" >&2; exit 1; }
echo "ok   server log: the cancelled download, then the whole file at the next login"
fi

if [[ " $SCENARIOS " == *" report "* ]]; then
echo "== log files and problem report"
python3 -I "$HERE/e2e/drive.py" report "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
fi

if [[ " $SCENARIOS " == *" update "* ]]; then
echo "== launcher self-update"
cp "$NEXT_BIN" "$STATE/release/mezeporta-bin"
"$HERE/sign-file.sh" "$STATE/release/mezeporta-bin" "$KEY" "$KEY_PASSWORD_FILE"
printf 'Test release for the e2e test.\n' > "$WORK/notes.txt"
python3 -I "$HERE/make_latest_json.py" "$NEXT_VERSION" "$WORK/notes.txt" "http://127.0.0.1:$RP" "$STATE/release" > "$STATE/release/latest.json"
python3 -I "$HERE/e2e/drive.py" update "$GAME" "$STATE" || { tail -n 40 "$WORK/driver.out" >&2; exit 1; }
[[ $(grep -c "^$RP GET /latest.json 200" "$STATE/requests.log") -ge 2 ]] || { echo "FAIL the restarted launcher did not check for updates again" >&2; exit 1; }
echo "ok   the restarted launcher checked latest.json again"
# hv8: the new launcher's log says it started after an update, and from which
# version (only when NEW_BIN is hv8 or later: older ones do not pass it on).
if [[ "${NEW_VERSION:-}" =~ -hv([0-9]+)$ ]] && (( BASH_REMATCH[1] >= 8 )); then
  grep -q "started after an update from $NEW_VERSION" "$GAME/Mezeporta/logs/mezeporta.log" \
    || { echo "FAIL mezeporta.log does not say \"started after an update from $NEW_VERSION\"" >&2; tail -n 20 "$GAME/Mezeporta/logs/mezeporta.log" >&2; exit 1; }
  echo "ok   mezeporta.log: started after an update from $NEW_VERSION"
fi
fi
echo "all e2e checks passed"
