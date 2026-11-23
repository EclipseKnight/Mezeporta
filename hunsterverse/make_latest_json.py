#!/usr/bin/env python3
"""Writes latest.json, the launcher's update manifest (src-tauri/src/hv_update.rs).

make_latest_json.py VERSION NOTES_FILE BASE_URL OUT_DIR > latest.json

OUT_DIR holds the signed binaries: mezeporta-bin(.sig) for linux-x86_64 and,
when built, Mezeporta.exe(.sig) for windows-i686. Each platform's url is
BASE_URL/<file>; its signature is the .sig file's content (base64 text).
pub_date is the newest binary's modification time (UTC).
"""
import datetime
import json
import os
import sys

PLATFORMS = {"linux-x86_64": "mezeporta-bin", "windows-i686": "Mezeporta.exe"}


def main():
    version, notes_file, base_url, out_dir = sys.argv[1:5]
    with open(notes_file, encoding="utf-8") as f:
        notes = f.read().strip()
    platforms, newest = {}, 0.0
    for key, name in PLATFORMS.items():
        path = os.path.join(out_dir, name)
        if not os.path.exists(path):
            continue
        with open(path + ".sig", encoding="ascii") as f:
            signature = f.read().strip()
        platforms[key] = {"url": base_url.rstrip("/") + "/" + name, "signature": signature}
        newest = max(newest, os.path.getmtime(path))
    if not platforms:
        sys.exit("no binaries in " + out_dir)
    pub_date = datetime.datetime.fromtimestamp(newest, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    json.dump({"version": version, "notes": notes, "pub_date": pub_date, "platforms": platforms},
              sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
