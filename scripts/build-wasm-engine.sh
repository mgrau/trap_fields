#!/bin/bash
# Build the in-browser engine: tf-app (geometry, meshing, BEM, analysis) + gmsh +
# OpenCASCADE as one threaded WebAssembly module, into frontend/public/engine/.
# Needs scripts/build-wasm-gmsh.sh first (Emscripten, threaded gmsh/OpenCASCADE).
#
# WASM64=1 builds the 64-bit-memory (Memory64) engine instead, tf_engine64.{js,wasm}, from
# the libraries of WASM64=1 scripts/build-wasm-gmsh.sh: up to 16 GB of memory instead of
# 4 GB. The page uses it where the browser supports Memory64, else tf_engine.{js,wasm}.
#
#   scripts/build-wasm-engine.sh && (cd frontend && npm run build)
#   # any static host that sends COOP/COEP headers can serve frontend/dist, e.g.
#   (cd frontend && npx vite preview)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
W=$root/wasm-build
if [ "${WASM64:-0}" = 1 ]; then
  TARGET=wasm64-unknown-emscripten; LIBDIR=gmsh-occ-mt64; NAME=tf_engine64
else
  TARGET=wasm32-unknown-emscripten; LIBDIR=gmsh-occ-mt; NAME=tf_engine
fi
[ -f "$W/$LIBDIR/libgmsh.a" ] || { echo "run ${WASM64:+WASM64=1 }scripts/build-wasm-gmsh.sh first"; exit 1; }
export EMSDK_PYTHON=${EMSDK_PYTHON:-$(command -v python3.14 || command -v python3.13 || command -v python3.12 || command -v python3)}
source "$W/emsdk/emsdk_env.sh" >/dev/null 2>&1

# isolated rustup (nightly: std is rebuilt with atomics for WebAssembly threads)
export RUSTUP_HOME=$W/rustup CARGO_HOME=$W/cargo
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  mkdir -p "$W/rustup-dl"
  arch=$(uname -m | sed 's/arm64/aarch64/')
  curl -sSfL "https://static.rust-lang.org/rustup/dist/$arch-apple-darwin/rustup-init" -o "$W/rustup-dl/rustup-init"
  chmod +x "$W/rustup-dl/rustup-init"
  "$W/rustup-dl/rustup-init" -y --no-modify-path --profile minimal --default-toolchain stable >/dev/null
fi
export PATH=$CARGO_HOME/bin:$PATH
# wasm32 is a tier-2 target with a prebuilt std; wasm64 (tier 3) only builds from rust-src
rustup toolchain install nightly --profile minimal --component rust-src >/dev/null
[ "$TARGET" = wasm64-unknown-emscripten ] || rustup target add --toolchain nightly "$TARGET" >/dev/null

# link flags live in crates/tf-wasm/build.rs; these are codegen flags for every crate
export CARGO_TARGET_DIR=$W/target-engine
export RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128 -C panic=abort"
cargo +nightly build -Z build-std=std,panic_abort --release -p tf-wasm --target $TARGET

out=$root/frontend/public/engine
# cargo names the script tf-engine.js, but the module refers to itself (pthread workers)
# by the name the linker produced (tf_engine.js), and to its .wasm as tf_engine.wasm:
# rename both for the wasm64 build
rel=$CARGO_TARGET_DIR/$TARGET/release
rm -f "$out/tf-engine.js"
sed -e "s/tf_engine\.js/$NAME.js/g" -e "s/tf_engine\.wasm/$NAME.wasm/g" "$rel/tf-engine.js" > "$out/$NAME.js"
# cargo copies the .wasm beside the script only for wasm32; for wasm64 it stays where the
# linker wrote it
wasm=$rel/tf_engine.wasm
[ -f "$wasm" ] || wasm=$(ls -t "$rel"/build/tf-wasm/*/out/tf_engine.wasm | head -1)
cp "$wasm" "$out/$NAME.wasm"
ls -la "$out"
