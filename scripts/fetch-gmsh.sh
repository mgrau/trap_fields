#!/usr/bin/env bash
# Download the official gmsh SDK (C API header + shared library with OpenCASCADE)
# into third_party/gmsh. Verifies the published SHA-256.
set -euo pipefail
VERSION="${GMSH_VERSION:-4.15.2}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  PLAT="MacOSARM" ; DIR="macOS" ;;
  Darwin-x86_64) PLAT="MacOSX"   ; DIR="macOS" ;;
  Linux-x86_64)  PLAT="Linux64"  ; DIR="Linux" ;;
  *) echo "unsupported platform $(uname -s)-$(uname -m)"; exit 1 ;;
esac
NAME="gmsh-${VERSION}-${PLAT}-sdk"
URL="https://gmsh.info/bin/${DIR}/${NAME}.tgz"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/third_party/gmsh"
if [[ -f "$DEST/include/gmshc.h" ]]; then echo "gmsh SDK already present in $DEST"; exit 0; fi
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
echo "downloading $URL"
curl -fsSL -o "$TMP/sdk.tgz" "$URL"
curl -fsSL -o "$TMP/sdk.tgz.sha256" "$URL.sha256"
EXPECTED="$(awk '{print $1}' "$TMP/sdk.tgz.sha256")"
ACTUAL="$(shasum -a 256 "$TMP/sdk.tgz" | awk '{print $1}')"
[[ "$EXPECTED" == "$ACTUAL" ]] || { echo "SHA-256 mismatch: $ACTUAL != $EXPECTED"; exit 1; }
mkdir -p "$TMP/x" && tar -xzf "$TMP/sdk.tgz" -C "$TMP/x"
mkdir -p "$ROOT/third_party" && rm -rf "$DEST" && mv "$TMP/x/$NAME" "$DEST"
echo "gmsh SDK installed in $DEST"; ls "$DEST/lib"
