#!/usr/bin/env bash
# build-release.sh: builds a Hunsterverse release of this fork (Linux x86_64
# and Windows i686, like upstream's release zips), signs the launcher binaries
# for the self-update, and writes the files of the GitHub release. Nothing is
# uploaded. Procedure and design: hunsterverse/README.md.
#
#   hunsterverse/build-release.sh [WORK]     (default /root/work/mezeporta-release)
#
# Output in WORK/out/ (upload all of it to the release tag v<version> of
# github.com/EclipseKnight/Mezeporta, or HV_RELEASE_REPO):
#   Mezeporta-Windows.zip, Mezeporta-Linux.zip   upstream v1.5.4's release zips with our binary
#                                                swapped in plus Mezeporta-hv-README.txt (installers)
#   Mezeporta.exe, mezeporta-bin                 the bare binaries the launcher's updater downloads
#   Mezeporta.exe.sig, mezeporta-bin.sig         their signatures (tauri signer, minisign)
#   latest.json                                  the update manifest (hv_update.rs)
#   Mezeporta-v<version>-source.zip              the source of this build (GPL-2.0)
#   SHA256SUMS
#
# Env: TAURI_PRIVATE_KEY_PATH (default ~/.secrets/mezeporta-updater.key),
#      HV_KEY_PASSWORD_FILE (default ~/.secrets/mezeporta-updater.key.password),
#      HV_RELEASE_REPO (default EclipseKnight/Mezeporta),
#      HV_RELEASE_BASE_URL (default the tag's download URL; the e2e test points it at a local server),
#      HV_SKIP_WINDOWS=1 (Linux only, for tests).
# Release notes: hunsterverse/release-notes/<version>.txt (required, a few short lines).
# Needs: git, node/npm, python3, curl, rustup toolchain 1.96.0 with target
# i686-pc-windows-msvc, cargo-xwin, clang, lld, libwebkit2gtk-4.1-dev and the
# other Tauri Linux build packages. Run through the shared lock on mhfz-dev:
#   flock /srv/devbot/.heavy-build.lock hunsterverse/build-release.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
WORK="$(realpath -m "${1:-/root/work/mezeporta-release}")"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$REPO/src-tauri/Cargo.toml" | head -n 1)"
CONF_VERSION="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["package"]["version"])' "$REPO/src-tauri/tauri.conf.json")"
[[ "$VERSION" == "$CONF_VERSION" ]] || { echo "Cargo.toml ($VERSION) and tauri.conf.json ($CONF_VERSION) versions differ" >&2; exit 1; }
TAG="v$VERSION"
RELEASE_REPO="${HV_RELEASE_REPO:-EclipseKnight/Mezeporta}"
BASE_URL="${HV_RELEASE_BASE_URL:-https://github.com/$RELEASE_REPO/releases/download/$TAG}"
KEY="${TAURI_PRIVATE_KEY_PATH:-$HOME/.secrets/mezeporta-updater.key}"
KEY_PASSWORD_FILE="${HV_KEY_PASSWORD_FILE:-$HOME/.secrets/mezeporta-updater.key.password}"
NOTES_FILE="$HERE/release-notes/$VERSION.txt"
SOURCE_NAME="Mezeporta-v$VERSION-source.zip"

UPSTREAM_DL=https://github.com/LilButter/Mezeporta/releases/download/v1.5.4
UPSTREAM_WINDOWS_SHA256=ea2431b8bbbc24208531c698c49fb141a221492596eaa04c202e17b0488f529f
UPSTREAM_LINUX_SHA256=5e75a37bf1de9745c41193b79ce2659b8fc386f7b12da9cabb5402caa94bee55

export RUSTUP_TOOLCHAIN=1.96.0
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$WORK/target"

for t in git npm python3 curl rustup; do
  command -v "$t" >/dev/null || { echo "$t is missing" >&2; exit 1; }
done
if [[ "${HV_SKIP_WINDOWS:-}" != 1 ]]; then
  for t in cargo-xwin clang lld-link; do command -v "$t" >/dev/null || { echo "$t is missing" >&2; exit 1; }; done
fi
[[ -f "$KEY" && -f "$KEY_PASSWORD_FILE" ]] || { echo "signing key or its password file is missing (see hunsterverse/README.md)" >&2; exit 1; }
[[ -f "$NOTES_FILE" ]] || { echo "write the release notes first: $NOTES_FILE" >&2; exit 1; }

mkdir -p "$WORK/upstream" "$WORK/out"
rm -f "$WORK/out/"*
cd "$REPO"

echo "== 1. frontend dependencies ($VERSION)"
npm ci --no-audit --no-fund --loglevel=error

echo "== 2. Linux build (x86_64-unknown-linux-gnu)"
# The vendored webkit2gtk-sys and javascriptcore-rs-sys ask pkg-config for the
# 4.0 names, which Ubuntu 24.04 no longer has; upstream's binary links 4.1, so
# alias them (shims in WORK only).
PCDIR="$WORK/pkgconfig-shims"
mkdir -p "$PCDIR"
for m in webkit2gtk javascriptcoregtk; do
  v="$(pkg-config --modversion "$m-4.1")"
  printf 'Name: %s-4.0\nDescription: alias of %s-4.1\nVersion: %s\nRequires: %s-4.1\n' "$m" "$m" "$v" "$m" > "$PCDIR/$m-4.0.pc"
done
PKG_CONFIG_PATH="$PCDIR${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" npx tauri build --ci -b none \
  --target x86_64-unknown-linux-gnu --config src-tauri/tauri.linux.conf.json -- --locked
cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/mezeporta" "$WORK/out/mezeporta-bin"

if [[ "${HV_SKIP_WINDOWS:-}" != 1 ]]; then
  echo "== 3. Windows build (i686-pc-windows-msvc, cargo-xwin)"
  # clang, not clang-cl: embed-resource preprocesses the .rc with "-xc -E".
  XWIN_ACCEPT_LICENSE=1 XWIN_ARCH=x86 XWIN_CROSS_COMPILER=clang npx tauri build --ci -b none \
    --runner cargo-xwin --target i686-pc-windows-msvc -- --locked
  cp "$CARGO_TARGET_DIR/i686-pc-windows-msvc/release/Mezeporta.exe" "$WORK/out/Mezeporta.exe"
fi

echo "== 4. signatures and latest.json"
BINARIES=(mezeporta-bin)
[[ "${HV_SKIP_WINDOWS:-}" == 1 ]] || BINARIES+=(Mezeporta.exe)
for b in "${BINARIES[@]}"; do
  "$HERE/sign-file.sh" "$WORK/out/$b" "$KEY" "$KEY_PASSWORD_FILE"
done
python3 -I "$HERE/make_latest_json.py" "$VERSION" "$NOTES_FILE" "$BASE_URL" "$WORK/out" > "$WORK/out/latest.json"

if [[ "${HV_SKIP_WINDOWS:-}" != 1 ]]; then
  echo "== 5. zips for the installers (upstream v1.5.4 zips, our binary swapped in)"
  for os in Windows Linux; do
    zip="$WORK/upstream/Mezeporta-$os.zip"
    want=$([[ $os == Windows ]] && echo "$UPSTREAM_WINDOWS_SHA256" || echo "$UPSTREAM_LINUX_SHA256")
    if [[ ! -f "$zip" || "$(sha256sum "$zip" | cut -d' ' -f1)" != "$want" ]]; then
      curl --silent --show-error --location --fail --retry 3 -o "$zip.part" "$UPSTREAM_DL/Mezeporta-$os.zip"
      mv -f "$zip.part" "$zip"
    fi
    [[ "$(sha256sum "$zip" | cut -d' ' -f1)" == "$want" ]] || { echo "upstream Mezeporta-$os.zip sha256 differs" >&2; exit 1; }
  done
  python3 -I "$HERE/pack_launcher_zip.py" "$WORK/upstream/Mezeporta-Windows.zip" Mezeporta.exe "$WORK/out/Mezeporta.exe" \
    "$HERE/Mezeporta-hv-README.txt" "$REPO/LICENSE" "$VERSION" "$WORK/out/Mezeporta-Windows.zip"
  python3 -I "$HERE/pack_launcher_zip.py" "$WORK/upstream/Mezeporta-Linux.zip" mezeporta-bin "$WORK/out/mezeporta-bin" \
    "$HERE/Mezeporta-hv-README.txt" "$REPO/LICENSE" "$VERSION" "$WORK/out/Mezeporta-Linux.zip"
  # Unzipped over an install, these two must come out executable (hv4).
  python3 -I - "$WORK/out/Mezeporta-Linux.zip" <<'EOF' || exit 1
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
bad = [n for n in ("mezeporta-bin", "run-mezeporta.sh") if z.getinfo(n).external_attr >> 16 & 0o111 != 0o111]
if bad:
    sys.exit("Mezeporta-Linux.zip: not executable when unzipped: %s" % ", ".join(bad))
EOF
fi

echo "== 6. source zip"
git -C "$REPO" ls-files -z --cached --others --exclude-standard \
  | python3 -I "$HERE/pack_source_zip.py" "$REPO" "Mezeporta-$VERSION" "$WORK/out/$SOURCE_NAME"
if [[ -n "$(git -C "$REPO" status --porcelain)" ]]; then
  echo "   note: the working tree has uncommitted changes; the source zip holds them as built"
fi

(cd "$WORK/out" && sha256sum -- * | grep -v ' SHA256SUMS$' > SHA256SUMS)
echo
echo "Built $WORK/out ($VERSION, update URLs under $BASE_URL):"
ls -l "$WORK/out"
cat "$WORK/out/SHA256SUMS"
