#!/usr/bin/env sh
# Starts the Mezeporta launcher on Linux (every start script runs this one).
# Hunsterverse (1.5.4-hv6): it also keeps what the launcher prints in
# Mezeporta/logs/run-mezeporta.log (the start before as run-mezeporta.1.log),
# so a launcher that never opens (a missing library, no display) still leaves
# a trace; the launcher's problem report includes it. Started from a terminal
# it shows the output as well. Replaces upstream's file in Mezeporta-Linux.zip
# (pack_launcher_zip.py).
set -eu
HERE="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
cd "$HERE"
LOG_DIR="$HERE/Mezeporta/logs"
LOG="$LOG_DIR/run-mezeporta.log"

if mkdir -p "$LOG_DIR" 2>/dev/null && { [ ! -e "$LOG" ] || mv -f "$LOG" "$LOG_DIR/run-mezeporta.1.log"; } \
   && : > "$LOG" 2>/dev/null; then
  {
    echo "== $(date -u '+%Y-%m-%d %H:%M:%S' 2>/dev/null) UTC run-mezeporta.sh"
    echo "kernel: $(uname -srm 2>/dev/null || true)"
    for f in /run/host/os-release /etc/os-release; do
      if [ -r "$f" ]; then echo "os ($f): $(sed -n 's/^PRETTY_NAME=//p' "$f" | tr -d '"')"; fi
    done
    if [ -r /.flatpak-info ]; then echo "flatpak: $(sed -n 's/^runtime=//p' /.flatpak-info | head -n 1)"; fi
    echo "display: DISPLAY=${DISPLAY:-} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} XDG_SESSION_TYPE=${XDG_SESSION_TYPE:-} GDK_BACKEND=${GDK_BACKEND:-}"
    echo "proton: MEZEPORTA_PROTON_CMD=${MEZEPORTA_PROTON_CMD:-} STEAM_COMPAT_DATA_PATH=${STEAM_COMPAT_DATA_PATH:-}"
    if [ ! -x "$HERE/mezeporta-bin" ]; then echo "mezeporta-bin is missing or not executable"; fi
  } >> "$LOG" 2>/dev/null || true
  if [ -t 1 ]; then
    # In a terminal: show the output and keep it.
    {
      code=0
      "$HERE/mezeporta-bin" "$@" 2>&1 || code=$?
      echo "== launcher exit code $code"
    } | tee -a "$LOG"
    exit 0
  fi
  # The launcher keeps its own log (mezeporta.log); here only what comes
  # before that file is open, and what libraries print.
  export MEZEPORTA_LOG_TO_STDOUT=0
  exec "$HERE/mezeporta-bin" "$@" >> "$LOG" 2>&1
fi
# No log folder could be written: start as before.
exec "$HERE/mezeporta-bin" "$@"
