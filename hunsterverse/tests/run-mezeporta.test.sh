#!/usr/bin/env bash
# Tests hunsterverse/run-mezeporta.sh (1.5.4-hv6) with a stand-in launcher:
#   bash hunsterverse/tests/run-mezeporta.test.sh
set -euo pipefail
SCRIPT="$(cd "$(dirname "$0")/.." && pwd)/run-mezeporta.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { echo "ok   $*"; }

game="$T/game with space"
mkdir -p "$game"
cp "$SCRIPT" "$game/run-mezeporta.sh"
# The stand-in prints to both streams and fails as a launcher does whose
# library is missing.
cat > "$game/mezeporta-bin" <<'EOF'
#!/bin/sh
echo "stand-in args: $* quiet=${MEZEPORTA_LOG_TO_STDOUT:-unset} cwd=$(pwd)"
echo "mezeporta-bin: error while loading shared libraries: libwebkit2gtk-4.1.so.0" >&2
exit 127
EOF
chmod +x "$game/mezeporta-bin" "$game/run-mezeporta.sh"
log="$game/Mezeporta/logs/run-mezeporta.log"

# 1. Not a terminal (menu entry, Steam): the output goes to the log, the exit
#    code is the launcher's.
code=0
(cd /tmp && "$game/run-mezeporta.sh" one two < /dev/null > "$T/out1" 2>&1) || code=$?
[[ $code == 127 ]] || fail "exit code $code, want 127"
[[ ! -s "$T/out1" ]] || fail "printed outside the log: $(cat "$T/out1")"
grep -q '^== .* UTC run-mezeporta.sh$' "$log" || fail "no header in $log"
grep -q '^kernel: ' "$log" || fail "no kernel line"
grep -q "stand-in args: one two quiet=0 cwd=$game" "$log" || fail "launcher stdout not in the log: $(cat "$log")"
grep -q 'libwebkit2gtk-4.1.so.0' "$log" || fail "launcher stderr not in the log"
ok "not a terminal: header, stdout and stderr in run-mezeporta.log, exit code 127 kept"

# 2. The next start keeps the one before as run-mezeporta.1.log.
"$game/run-mezeporta.sh" three < /dev/null > /dev/null 2>&1 || true
grep -q 'stand-in args: one two' "$game/Mezeporta/logs/run-mezeporta.1.log" || fail "previous start not kept"
grep -q 'stand-in args: three' "$log" || fail "new start not logged"
! grep -q 'stand-in args: one two' "$log" || fail "old start still in the current log"
"$game/run-mezeporta.sh" four < /dev/null > /dev/null 2>&1 || true
[[ $(find "$game/Mezeporta/logs" -name 'run-mezeporta*.log' | wc -l) == 2 ]] || fail "more than two script logs"
ok "rotation: current and one previous start"

# 3. In a terminal: shown and kept.
if command -v script > /dev/null; then
  script -qec "\"$game/run-mezeporta.sh\" tty" /dev/null > "$T/out3" 2>&1 || true
  grep -q 'stand-in args: tty quiet=unset' "$T/out3" || fail "terminal output missing: $(cat "$T/out3")"
  grep -q 'stand-in args: tty' "$log" || fail "terminal run not logged"
  grep -q '== launcher exit code 127' "$log" || fail "terminal run: no exit code line"
  ok "terminal: output shown and kept, with the exit code"
else
  echo "skip terminal case (no 'script')"
fi

# 4. A log folder that cannot be made still starts the launcher.
rm -rf "$game/Mezeporta"
printf 'not a folder' > "$game/Mezeporta"
code=0
"$game/run-mezeporta.sh" five < /dev/null > "$T/out4" 2>&1 || code=$?
[[ $code == 127 ]] || fail "no log folder: exit code $code"
grep -q 'stand-in args: five quiet=unset' "$T/out4" || fail "no log folder: launcher not started: $(cat "$T/out4")"
ok "no log folder: started as before"
