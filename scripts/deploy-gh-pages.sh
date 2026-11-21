#!/usr/bin/env bash
# Build the browser-only app (the WebAssembly engine, no server) and publish it on the
# gh-pages branch of the `origin` remote, served at https://<user>.github.io/<repo>/.
#
# Needs the engine in frontend/public/engine/ (scripts/build-wasm-engine.sh). The branch
# holds one commit, replaced on every deploy (the engine is 25 MB; no history is kept).
# TF_PAGES_BASE overrides the base path (default: /<repo name>/).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

remote="$(git remote get-url origin)"
repo="$(basename -s .git "$remote")"
base="${TF_PAGES_BASE:-/$repo/}"
test -f frontend/public/engine/tf_engine.wasm || { echo "no engine: run scripts/build-wasm-engine.sh first" >&2; exit 1; }
# the 64-bit engine is optional: without it every browser runs the 32-bit one
test -f frontend/public/engine/tf_engine64.wasm || echo "note: no 64-bit engine (WASM64=1 scripts/build-wasm-engine.sh); publishing the 32-bit one only" >&2

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
(cd frontend && npx tsc -b && TF_BASE="$base" VITE_STATIC=1 npx vite build --outDir "$out/site" --emptyOutDir)
touch "$out/site/.nojekyll" # serve files as they are (no Jekyll processing)

src="$(git rev-parse --short HEAD)"
cd "$out/site"
git init -q -b gh-pages
git add -A
git -c user.name="$(git -C "$root" config user.name)" -c user.email="$(git -C "$root" config user.email)" \
  commit -q -m "Browser-only build of $src (base $base)"
git push -q -f "$remote" gh-pages
echo "published $src to gh-pages (base $base)"
