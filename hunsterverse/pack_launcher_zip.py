#!/usr/bin/env python3
"""Builds our launcher zip from upstream's release zip, for build-release.sh.

pack_launcher_zip.py UPSTREAM_ZIP BINARY_NAME NEW_BINARY README LICENSE VERSION OUT_ZIP

Copies every entry of UPSTREAM_ZIP in its order, with its bytes, compression
and attributes, except BINARY_NAME, whose content becomes NEW_BINARY (same
name and attributes, deflated). Adds README (with {version} filled in)
followed by LICENSE (the GPL text), CRLF, as Mezeporta-hv-README.txt. Same
inputs give the same bytes (fixed dates).

Linux zip (a BINARY_NAME without extension, mezeporta-bin): that binary and
every *.sh get Unix mode 755 (1.5.4-hv4). Upstream's zip is made on Windows
and stores no Unix modes, so unzipping it over an install (the Play page's
"Updating the launcher") left run-mezeporta.sh, which the start scripts run,
not executable: the launcher did not open. Since 1.5.4-hv6 its
run-mezeporta.sh is ours (this folder's, which also keeps a log file).
"""
import os
import sys
import zipfile

DATE = (2026, 11, 23, 0, 0, 0)
RUN_SCRIPT = "run-mezeporta.sh"
OUR_RUN_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), RUN_SCRIPT)
README_NAME = "Mezeporta-hv-README.txt"
OLD_README_NAMES = {"Mezeporta-hv1-README.txt"}


def executable(info):
    """info as a Unix file with mode 755 (the DOS attribute bits kept)."""
    info.create_system = 3
    info.external_attr = (0o100755 << 16) | (info.external_attr & 0xFFFF)
    return info


def main():
    upstream, binary_name, new_binary, readme, licence, version, out = sys.argv[1:8]
    replaced = False
    unix = "." not in binary_name
    with zipfile.ZipFile(upstream) as src, zipfile.ZipFile(out, "w") as dst:
        for info in src.infolist():
            if info.filename == README_NAME or info.filename in OLD_README_NAMES:
                continue
            if info.filename == binary_name:
                new = zipfile.ZipInfo(info.filename, DATE)
                new.external_attr = info.external_attr
                new.create_system = info.create_system
                new.compress_type = zipfile.ZIP_DEFLATED
                if unix:
                    executable(new)
                with open(new_binary, "rb") as f:
                    dst.writestr(new, f.read(), compresslevel=9)
                replaced = True
            else:
                data = src.read(info.filename)
                if unix and info.filename == RUN_SCRIPT:
                    with open(OUR_RUN_SCRIPT, "rb") as f:
                        data = f.read()
                    info.file_size = len(data)
                if unix and info.filename.endswith(".sh"):
                    executable(info)
                dst.writestr(info, data)
        if not replaced:
            sys.exit(f"{binary_name} not in {upstream}")
        with open(readme, encoding="utf-8") as f:
            text = f.read().replace("{version}", version)
        with open(licence, encoding="utf-8") as f:
            text += "\n" + f.read()
        text = text.replace("\r\n", "\n").replace("\n", "\r\n")
        info = zipfile.ZipInfo(README_NAME, DATE)
        info.external_attr = 0o644 << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        dst.writestr(info, text.encode("utf-8"), compresslevel=9)


if __name__ == "__main__":
    main()
