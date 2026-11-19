#!/bin/bash
# Build the in-browser engine: tf-app (geometry, meshing, BEM, analysis) + gmsh +
# OpenCASCADE as one threaded WebAssembly module, into frontend/public/engine/.
# Needs scripts/build-wasm-gmsh.sh first (Emscripten, threaded gmsh/OpenCASCADE).
#
#   scripts/build-wasm-engine.sh && (cd frontend && npm run build)
#   # any static host that sends COOP/COEP headers can serve frontend/dist, e.g.
#   (cd frontend && npx vite preview)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
W=$root/wasm-build
[ -f "$W/gmsh-occ-mt/libgmsh.a" ] || { echo "run scripts/build-wasm-gmsh.sh first"; exit 1; }
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
rustup toolchain install nightly --profile minimal --component rust-src --target wasm32-unknown-emscripten >/dev/null

# link flags live in crates/tf-wasm/build.rs; these are codegen flags for every crate
export CARGO_TARGET_DIR=$W/target-engine
export RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128 -C panic=abort"
cargo +nightly build -Z build-std=std,panic_abort --release -p tf-wasm --target wasm32-unknown-emscripten

out=$root/frontend/public/engine
# cargo names the script tf-engine.js, but the module refers to itself (pthread workers)
# by the name the linker produced: tf_engine.js
rel=$CARGO_TARGET_DIR/wasm32-unknown-emscripten/release
rm -f "$out/tf-engine.js"
cp "$rel/tf-engine.js" "$out/tf_engine.js"
cp "$rel/tf_engine.wasm" "$out/"
ls -la "$out"
