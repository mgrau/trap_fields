#!/bin/bash
# Build the Rust solver benchmark (crates/tf-bench) as multithreaded WebAssembly:
# Emscripten pthreads + nightly Rust with a rebuilt std (+atomics), and mimalloc
# (Emscripten's default allocator serializes threads). Toolchains live in the git-ignored
# wasm-build/ (run scripts/build-wasm-gmsh.sh first for emsdk).
#
#   scripts/build-wasm-solver.sh
#   node wasm-build/out/node-mt/tf-bench.js mesh.bin --nodes mesh.nodes.f64 --method compressed --threads 16
#   node wasm/serve.mjs wasm-build/out 8790     # COOP/COEP headers; open /bench.html
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
W=$root/wasm-build
[ -d "$W/emsdk" ] || { echo "run scripts/build-wasm-gmsh.sh first (installs Emscripten)"; exit 1; }
export EMSDK_PYTHON=${EMSDK_PYTHON:-$(command -v python3.14 || command -v python3.13 || command -v python3.12 || command -v python3)}
source "$W/emsdk/emsdk_env.sh" >/dev/null 2>&1

# isolated rustup: does not touch ~/.rustup, ~/.cargo or PATH outside this script
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

common="-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128 -C panic=abort \
 -C link-arg=-pthread -C link-arg=-sMALLOC=mimalloc -C link-arg=-sPTHREAD_POOL_SIZE=navigator.hardwareConcurrency+2 \
 -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sMAXIMUM_MEMORY=4GB -C link-arg=-sINITIAL_MEMORY=64MB -C link-arg=-sSTACK_SIZE=8MB"
build() { # name, extra link flags
  CARGO_TARGET_DIR=$W/target-$1 RUSTFLAGS="$common $2" \
    cargo +nightly build -Z build-std=std,panic_abort --release -p tf-bench --target wasm32-unknown-emscripten
  mkdir -p "$W/out/$1"
  cp "$W/target-$1/wasm32-unknown-emscripten/release/tf-bench.js" "$W/target-$1/wasm32-unknown-emscripten/release/tf_bench.wasm" "$W/out/$1/"
}
# Node: reads files directly
build node-mt "-C link-arg=-sNODERAWFS=1 -C link-arg=-sEXIT_RUNTIME=1"
# browser: loaded from the page; main() runs on a pthread so the page stays responsive
build mt "-C link-arg=-sPROXY_TO_PTHREAD=1 -C link-arg=-sENVIRONMENT=web,worker -C link-arg=-sMODULARIZE=1 \
 -C link-arg=-sEXPORT_NAME=TfBenchMT -C link-arg=-sINVOKE_RUN=0 -C link-arg=-sEXIT_RUNTIME=0 \
 -C link-arg=-sEXPORTED_RUNTIME_METHODS=FS,callMain"
cp "$root/wasm/bench.html" "$root/wasm/bench-worker.js" "$W/out/"
for m in "$W"/bench/mesh_*.bin "$W"/bench/mesh_*.nodes.f64; do [ -f "$m" ] && cp "$m" "$W/out/"; done
ls -la "$W/out/mt" "$W/out/node-mt"
