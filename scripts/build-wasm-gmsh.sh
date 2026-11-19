#!/bin/bash
# Build gmsh 4.15.2 + OpenCASCADE 7.9.3 for WebAssembly (Emscripten), with -pthread so they
# can be linked into the threaded engine (scripts/build-wasm-engine.sh), plus the
# STEP-meshing test in wasm/. Everything goes into wasm-build/ (git-ignored). ~6 min.
#
#   scripts/build-wasm-gmsh.sh            # toolchain, OCCT, gmsh, wasm/mesh_step.{js,wasm} (browser) + node test
#   node wasm-build/out/mesh_step_node.js trap_electrodes_nothread.step
#   node wasm/serve.mjs wasm-build/out 8790   # then open http://127.0.0.1:8790
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
B=$root/wasm-build
J=${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc)}
GMSH=4.15.2
OCCT=V7_9_3
mkdir -p "$B/src" "$B/out"

# --- Emscripten (needs Python >= 3.10)
if [ ! -d "$B/emsdk" ]; then
  git clone --depth 1 https://github.com/emscripten-core/emsdk.git "$B/emsdk"
fi
export EMSDK_PYTHON=${EMSDK_PYTHON:-$(command -v python3.14 || command -v python3.13 || command -v python3.12 || command -v python3)}
(cd "$B/emsdk" && ./emsdk install latest && ./emsdk activate latest) >/dev/null
source "$B/emsdk/emsdk_env.sh" >/dev/null 2>&1

# --- sources
cd "$B/src"
[ -d gmsh-$GMSH-source ] || { curl -sSfLO https://gmsh.info/src/gmsh-$GMSH-source.tgz; tar xzf gmsh-$GMSH-source.tgz; }
[ -d OCCT-${OCCT#V} ] || { curl -sSfL https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/$OCCT.tar.gz -o occt.tar.gz; tar xzf occt.tar.gz; }

# --- OpenCASCADE: static, no FreeType/Tk/Draw. Native WebAssembly exceptions
# (-fwasm-exceptions; wins over the -fexceptions OCCT's CMake adds), as Rust's Emscripten
# target uses them and the two exception models cannot be linked together.
# OCC_CONVERT_SIGNALS is cancelled: it puts setjmp inside try blocks (OCC_CATCH_SIGNALS),
# which LLVM's wasm exception lowering turns into invalid code (seen in
# ShapeUpgrade_ShapeDivide::Perform), and there are no signals to convert in WebAssembly.
mkdir -p "$B/occt-build-mt" && cd "$B/occt-build-mt"
if [ ! -f "$B/occt-install-mt/lib/libTKDESTEP.a" ]; then
  cmake -G "Unix Makefiles" -DCMAKE_TOOLCHAIN_FILE="$EMSDK/upstream/emscripten/cmake/Modules/Platform/Emscripten.cmake" \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_LIBRARY_TYPE=Static -DINSTALL_DIR="$B/occt-install-mt" -DINSTALL_DIR_INCLUDE=inc \
    -DINSTALL_DIR_RESOURCE=src -DUSE_FREETYPE=OFF -DUSE_TK=OFF -DBUILD_MODULE_Visualization=ON \
    -DBUILD_MODULE_ApplicationFramework=ON -DBUILD_MODULE_DataExchange=ON -DBUILD_MODULE_DETools=OFF \
    -DBUILD_MODULE_Draw=OFF -DBUILD_DOC_Overview=OFF -DCMAKE_C_FLAGS="-pthread -fwasm-exceptions -UOCC_CONVERT_SIGNALS" \
    -DCMAKE_CXX_FLAGS="-pthread -fwasm-exceptions -UOCC_CONVERT_SIGNALS" \
    "$B/src/OCCT-${OCCT#V}" >/dev/null
  make -j"$J" >/dev/null && make install >/dev/null
fi

# --- gmsh: mesh module + MathEval + OCC, same threads and exception model as OCCT
mkdir -p "$B/gmsh-occ-mt" && cd "$B/gmsh-occ-mt"
CASROOT="$B/occt-install-mt" emcmake cmake "$B/src/gmsh-$GMSH-source" -DCMAKE_BUILD_TYPE=Release -DDEFAULT=0 \
  -DENABLE_BUILD_LIB=1 -DENABLE_MESH=1 -DENABLE_MATHEX=1 -DENABLE_ANN=1 -DENABLE_EIGEN=1 -DENABLE_PARSER=1 \
  -DENABLE_GMP=0 -DENABLE_TINYOBJLOADER=1 -DENABLE_OPENMP=0 -DENABLE_FLTK=0 -DENABLE_OCC=1 -DENABLE_OCC_STATIC=1 \
  -DENABLE_OCC_CAF=0 -DCMAKE_CXX_FLAGS="-fwasm-exceptions -pthread" -DCMAKE_C_FLAGS="-fwasm-exceptions -pthread" \
  -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=BOTH -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=BOTH | grep "Build options"
emmake make lib -j"$J" >/dev/null

# --- link the test program (node and browser variants)
L=$B/occt-install-mt/lib
LIBS=()
for t in TKDESTEP TKDEIGES TKDE TKXCAF TKVCAF TKCAF TKLCAF TKCDF TKV3d TKService TKXSBase TKOffset TKFeat TKFillet \
         TKBool TKMesh TKHLR TKBO TKPrim TKShHealing TKTopAlgo TKGeomAlgo TKBRep TKGeomBase TKG3d TKG2d TKMath TKernel; do
  LIBS+=("$L/lib$t.a")
done
emcc -O2 -fwasm-exceptions -pthread -c "$root/wasm/mesh_step.c" -I"$B/src/gmsh-$GMSH-source/api" -o "$B/out/mesh_step.o"
link() {
  em++ -O2 -fwasm-exceptions -pthread "$B/out/mesh_step.o" "$B/gmsh-occ-mt/libgmsh.a" -Wl,--start-group "${LIBS[@]}" -Wl,--end-group \
    -sINITIAL_MEMORY=64MB -sALLOW_MEMORY_GROWTH=1 -sMAXIMUM_MEMORY=4GB -sSTACK_SIZE=16MB "$@" 2>&1 | grep -v warning || true
}
link -o "$B/out/mesh_step_node.js" -sEXIT_RUNTIME=1 -sNODERAWFS=1
link -o "$B/out/mesh_step.js" -sENVIRONMENT=web,worker -sMODULARIZE=1 -sEXPORT_NAME=GmshModule -sINVOKE_RUN=0 \
  -sEXIT_RUNTIME=0 -sEXPORTED_RUNTIME_METHODS=FS,callMain
cp "$root/wasm/index.html" "$root/wasm/bench.html" "$root/wasm/bench-worker.js" "$B/out/"
[ -f "$root/trap_electrodes_nothread.step" ] && cp "$root/trap_electrodes_nothread.step" "$B/out/"
ls -la "$B/out"/mesh_step*.wasm
