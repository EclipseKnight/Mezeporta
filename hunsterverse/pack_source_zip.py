#!/usr/bin/env python3
"""Zips the source of a build, for build-release.sh.

git ls-files -z --cached --others --exclude-standard | pack_source_zip.py REPO PREFIX OUT_ZIP

Every listed file that exists (deleted files are skipped) goes in under
PREFIX/, sorted, deflated, with a fixed date, so the same tree gives the same
bytes. Takes the working tree as it is, so the zip matches what was built even
before the change is committed.
"""
import os
import sys
import zipfile

DATE = (2026, 11, 23, 0, 0, 0)


def main():
    repo, prefix, out = sys.argv[1:4]
    names = sorted(n for n in sys.stdin.buffer.read().decode("utf-8").split("\0") if n)
    with zipfile.ZipFile(out, "w") as dst:
        for name in names:
            path = os.path.join(repo, name)
            if not os.path.isfile(path) or os.path.islink(path):
                continue
            info = zipfile.ZipInfo(prefix + "/" + name, DATE)
            mode = 0o755 if os.access(path, os.X_OK) else 0o644
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as f:
                dst.writestr(info, f.read(), compresslevel=9)


if __name__ == "__main__":
    main()
