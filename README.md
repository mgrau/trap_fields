# trapfields

Fields, potentials and pseudopotentials of realistic RF Paul traps, starting from a CAD
(STEP) model of the electrodes.

**Try it in the browser:** https://mgrau.github.io/trap_fields/ — the whole application
running locally in WebAssembly (no server; your files stay in your browser).

Status: load a STEP export, inspect and measure it, remove holes and threads (click a
face; OpenCASCADE defeaturing), orient it into the simulation frame,
label electrodes, generate a graded surface mesh, solve for one basis potential per
electrode (with its solid-harmonic expansion about the trap center), and analyze the
trap for any RF/DC drive: pseudopotential, equilibrium, secular modes, Mathieu
parameters, Floquet stability, anharmonicity, depth, slices, sweeps of the drive, and the
equilibrium and normal modes of N-ion chains. A convergence study refines the mesh and
extrapolates every result. Everything runs either on a native server or entirely in the
browser (WebAssembly).

The mesh, the solution and the results computed from it (sweeps, chains, slices, the last
convergence study) are saved in the project folder — in IndexedDB for the browser
engine — and restored when the GUI restarts, as long as what they depend on is unchanged.
Geometry → Download project packs the project (trapfields.json, STEP file, mesh/,
solve/, results/) into one .tar; Open project loads such a bundle.

## Layout

```
crates/tf-gmsh    safe Rust wrapper around the gmsh C API (gmsh + OpenCASCADE do STEP import and meshing)
crates/tf-geom    STEP name/unit recovery, rigid transforms, display tessellation, graded BEM mesh
crates/tf-bem     BEM electrostatics: panel integrals, dense LU and compressed (H-matrix + GMRES)
                  solvers, solid-harmonic expansions, trap analysis, ion chains
crates/tf-app     the application core: project state and one command interface
crates/tf-server  `trapfields` binary: local HTTP API over tf-app + serves the GUI
crates/tf-wasm    tf-app as a WebAssembly engine for the browser
frontend/         TypeScript + React + three.js GUI
scripts/          fetch-gmsh.sh (gmsh SDK into third_party/), WebAssembly builds
```

## Setup

Requires Rust (stable), Node 18+, and the gmsh SDK:

```sh
scripts/fetch-gmsh.sh            # gmsh 4.15.2 SDK -> third_party/gmsh (or set GMSH_DIR)
(cd frontend && npm install && npm run build)
cargo build --release
```

## Run

```sh
cargo run --release -- gui /path/to/project --open      # http://127.0.0.1:8765
```

The project directory holds your STEP files; the GUI writes `trapfields.json`
(selected file, transforms, removed features, electrode labels, mesh settings) and `mesh/` there.
The STEP file itself is never modified: transforms and feature removals (Geometry → Remove
features) are re-applied after every import. A removal names its body and identifies each
face by centroid and area, measured on the model as it was when the removal was made.

Frontend development with hot reload: run the server as above, then
`cd frontend && npm run dev` and open http://localhost:5173 (API calls are proxied).

### In the browser only

```sh
scripts/build-wasm-gmsh.sh       # once: gmsh + OpenCASCADE for Emscripten (~6 min)
scripts/build-wasm-engine.sh     # the app core -> frontend/public/engine/
(cd frontend && npm run build && npm run preview)    # http://localhost:4173
```

The GUI uses the native server when one answers at `/api`, else starts the engine in a
web worker (`?engine=browser` / `?engine=server` force a choice; the status bar shows
which one runs). The engine's project folder lives in the browser's IndexedDB; STEP files
come in through Upload or drag and drop. WebAssembly threads need cross-origin isolation:
the host sends `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`, or — on hosts without custom headers, like
GitHub Pages — a static build (`VITE_STATIC=1`) adds them with a service worker
(`public/coi-sw.js`, registered before the app starts, one reload on the first visit).

`scripts/deploy-gh-pages.sh` builds that static app for the `origin` repository's path
(`TF_BASE=/<repo>/`) and publishes it as the single commit of its gh-pages branch.

### Workspace

The top bar holds the workflow steps (Geometry → Study, each marked done, needing
attention or to do), the running job with its Stop button, the layout (3D, Split with a
draggable divider, Results) and the units. The sidebar holds the inputs; figures and
tables go in the results area beside the 3D view, whose own controls float over it. The
Trap and Study tabs look at the trap region: the model is clipped to a box around the
trap center (Trap region, its size, Frame), seen down the trap axis on the first visit;
the model tabs keep their own camera. While a job runs, a thin bar under the top bar
shows how far it is where that can be told (GMRES residual against its tolerance, study
levels, bodies and faces of a load) and sweeps otherwise.

Long jobs (load, mesh, solve, study) can be stopped: solves stop within one GMRES
iteration (or after the LU factorization), loads between their stages and bodies/faces,
meshing after its current gmsh pass; a stopped study keeps its finished levels. A single
gmsh call (the OpenCASCADE import of a big STEP file, a meshing pass) cannot be
interrupted: if a load or mesh does not stop within a few seconds, the engine (browser)
or the server (native, Unix) restarts, with the project as it was before the job, and
re-opens the project's STEP file unless that is the file that would not stop loading.
STEP files over 10 MB ask before loading.

Every plot exports: copy the image or the data (tab-separated), or save PNG, SVG or CSV,
in the dark theme or with a light background; tables copy or save as CSV.

## Exporting from Fusion 360

- One **solid body per electrode piece**, named (e.g. `RF1`, `DC3`, `Endcap 1`). Names are
  read from the STEP file; bodies can also be grouped into electrodes in the GUI.
- File → Export → STEP. Units are taken from the file (mm is typical).
- Make threads **cosmetic** (Thread feature, uncheck *Modeled*). Modeled threads are
  heavy and can fail to mesh (`Impossible to mesh periodic surface`); such faces are
  skipped and reported.

## Coordinates and units

Everything internal is SI (meters). The *trap frame* is the STEP frame after the rigid
transforms listed in `trapfields.json`; meshes and (later) field expansions are in this
frame. Put the trap center at the origin (Geometry → Orientation).

## Mesh output

`mesh/surface_mesh.bin` (little-endian): `"TFM1"`, `u32 n_nodes`, `u32 n_tri`,
`f32 nodes[3·n_nodes]` (m, trap frame), `u32 tri[3·n_tri]`, `u32 tri_body[n_tri]`,
`u32 tri_face[n_tri]`. `mesh/surface_mesh.json` holds body names, labels, sizing
parameters and statistics.

## Meshing notes

Panels on curved faces are capped at about one radius of curvature (6 per circle).
Without that, gmsh folds or duplicates triangles on small curved faces (e.g. 1 mm holes)
when the panel size exceeds the hole size. Every mesh is checked for duplicate and folded
triangles; defective faces are remeshed finer, and the solver refuses a mesh that still
has defects.

## BEM solver

Free-space collocation BEM with piecewise-constant charge density on flat triangles
(closed-form near-field integrals, 7-point quadrature far away). Floating electrodes
carry zero net charge. One basis solution per non-floating electrode: 1 V on it, 0 V on
the others. The potential for any set of voltages is the weighted sum of the basis
solutions. Two linear solvers (Solve tab; "auto" picks compressed from 5,000 panels):

- **dense**: full matrix and in-place LU (faer): exact, 8·N² bytes, time ∝ N³
  (17k panels: 2.4 GB, ~6 s on an M5 Max).
- **compressed**: H-matrix — cluster tree, far-field blocks (η = 2) compressed by adaptive
  cross approximation to 1e-6, near field dense — and GMRES (tolerance 1e-9, block-Jacobi
  preconditioner, all electrodes in lockstep). Floating electrodes by superposition. On the
  trap: 22 % of the dense storage, 48 iterations, 0.9 s for 17k panels, agreeing with
  dense to ~1e-7 in fields, curvatures and secular frequencies.

Expansions are computed directly from the surface charges (no fitting):

    φ(r) = Σ_{l≤L} Σ_{m=-l..l} a_lm · Y_lm((r − c)/r0)

with real, Racah-normalized regular solid harmonics (no Condon–Shortley phase):
Y_l0 = rˡ P_l(cos θ), Y_lm = √(2 (l−m)!/(l+m)!) rˡ P_l^m(cos θ) cos mφ for m > 0 and
sin |m|φ for m < 0. Low orders: Y₁ = (x, y, z) for m = (1, −1, 0);
Y₂,₀ = z² − (x²+y²)/2, Y₂,₁ = √3 xz, Y₂,₋₁ = √3 yz, Y₂,₂ = (√3/2)(x²−y²), Y₂,₋₂ = √3 xy.
Coefficients a_lm are in volts at flat index l² + l + m. The series converges inside the
sphere of radius r_min (distance from c to the nearest electrode); by default r0 = r_min.

Outputs in `solve/`: `solution.json` (per basis: coefficients, potential, gradient and
Hessian at the center, induced charges = capacitance matrix) and `sigma.bin`
(`"TFS1"`, `u32 n_basis`, `u32 n_tri`, `u32 0`, `f64 σ[n_basis][n_tri]` in C/m², triangle
order of `mesh/surface_mesh.bin`). `POST /api/solve/eval` evaluates φ and ∇φ at arbitrary
points for given electrode voltages.

Field export (Solve → Export fields, `POST /api/solve/fields`): φ and E = −∇φ on a
regular grid (center, half-widths and nodes per axis; one node makes a plane or a line;
at most 125,000 points) as CSV, from the surface charges, so valid everywhere; an `inside`
column marks points inside an electrode, where φ is its potential. *Per electrode* gives
every basis solution (`phi_<electrode>_V`, `Ex_<electrode>_V_per_m`, … at 1 V on it, 0 V
on the others), so any drive is their weighted sum; *Trap drive* gives the RF amplitude and
static potentials of the Trap tab's voltages (`phi_rf_V`, `Ex_rf_V_per_m`, …, `phi_dc_V`, …)
and U in eV. Rows run x fastest, then y, then z:
`np.genfromtxt('fields_basis.csv', delimiter=',', names=True)` reads it. 21³ points take
about a second natively on the 25k-panel trap.

Validation (`cargo test -p tf-bem`): sphere capacitance (−0.08 % at 5120 panels), two-sphere
capacitance matrix vs. the bispherical series (0.1–0.2 %), floating-conductor neutrality,
expansion vs. direct evaluation (7 digits). Mesh-quality and residual diagnostics:
`cargo run --release -p tf-bem --example diagnose -- mesh/surface_mesh.bin`.

## Trap analysis (Trap tab)

RF electrodes carry V_e cos Ωt (amplitude V_e, zero to peak; negative = opposite phase),
DC electrodes static voltages. With φ_rf the RF amplitude potential and φ_dc the static
one, the ion moves in

    U(r) = Q² |∇φ_rf|² / (4 m Ω²) + Q φ_dc(r)

Computed from the expansions (milliseconds, re-run on every input change):

- RF null (least-squares zero of ∇φ_rf; a null *line* is detected), equilibrium (damped
  Newton on U inside the expansion's convergence sphere), offset and RF field at the ion
  (excess micromotion).
- Secular modes: eigenvectors/values of ∇∇U at the equilibrium; f = √(k/m)/2π.
- Exact linear check: Floquet multipliers of ẍ = −(Q/m)(H_dc + H_rf cos Ωt)x over one RF
  period (6×6 monodromy matrix); stability and frequencies without the adiabatic
  approximation.
- Mathieu a, q on the principal axes of the RF quadrupole (Hessian of φ_rf):
  q = −2Qλ_rf/(mΩ²), a = 4Q uᵀH_dc u/(mΩ²).
- Anharmonicity: degree-6 fit of U along each mode axis; L₃ = c2/|c3|, L₄ = √(c2/|c4|)
  are the amplitudes at which the cubic/quartic terms equal the harmonic one.

From the surface charges directly (valid everywhere outside the electrodes, ~1 s):
trap depth, U along each mode axis against ½mω²x², and energy slices through the ion (a 2D
map with the ion, the RF null, chain ions and the principal axes of U, and a plane in the
3D view). Within (10⁻⁴)^(1/(l_max+1))·r_valid of the expansion center a slice uses the
expansion instead (≲10⁻⁵ eV from the direct sum, ~100× faster), blended into the direct
sum farther out, so zoomed-in slices default to a fine grid (up to 401²); the maps are
drawn with bilinear interpolation and anti-aliased contours.

Trap depth: the lowest barrier over all escape routes, not only straight ones. U is
sampled on a 29³ grid centered on the ion, spaced r_min/8 there and stretched
geometrically out to ±10 r_min; a priority flood from the ion (the cost of a route is
its highest U; 26 neighbours) finds the route to an electrode or to the edge of the grid
whose highest point is lowest. When that point lies between the electrodes it is a
saddle of U, and Newton's method on ∇U = 0 locates it off the grid, so the depth does
not depend on the grid (25³ to 41³ agree to 10⁻⁹ eV on the four-blade trap, whose
saddle is 7 % below the lowest of 400 straight rays). The route and the saddle are drawn
in the 3D view.

Stability diagram: a against |q| of each RF quadrupole axis on the Mathieu stability
chart (first region between a₀(q) and b₁(q), iso-β lines; characteristic values from
the tridiagonal Fourier matrices), with the path of the last sweep.

Sweeps: the single-ion analysis over the RF amplitude, the RF frequency or one
electrode's DC voltage; modes keep their identity by the axis they point along, and |q|
is plotted against the adiabatic limit.

Ion chains: N ions in U plus their Coulomb repulsion; damped Newton on all 3N coordinates
from a linear chain along the weakest axis (zigzags are found when it is unstable), and
the normal modes from the 3N×3N Hessian with their weight along each principal axis.

`POST /api/trap`, `/api/trap/slice`, `/api/sweep` and `/api/chain` expose the same analysis.

## Convergence study (Study tab)

Meshes and solves at several refinements s (h_min and h_max scaled by s, the grading
radii fixed) without touching the current mesh and solution, tracks the potential, field
and curvature of every electrode at the center and the secular frequencies, and
extrapolates each from the three finest levels, f(s) ≈ f∞ + C sᵖ, with the error of the
finest level. GMRES residuals and finished levels stream in while it runs.

The grading slope (h_max − h_min)/(d_max − d_min) matters most. On the four-blade trap
(¹³⁸Ba⁺, 10 MHz, ±100 V, endcaps 10 V):

| d_max | panels at s = 1 | axial f at s = 1 / 0.7 / 0.5 | extrapolated |
|---|---|---|---|
| 7.5 mm (slope 0.43) | 17k | 68.5 / 67.4 / 66.6 kHz | 64.2 kHz |
| 19 mm (0.15, suggested) | 25k | 66.2 / 65.8 / 65.5 kHz | 64.5 kHz |
| 40 mm (0.07) | 51k | 65.5 / 65.3 / 65.2 kHz | 64.7 kHz |

The radial frequencies extrapolate to 241.3–241.5 and 279.8 kHz under all three. The
axial frequency comes from the endcaps 3.5 mm away, which the steep grading meshes with
~1 mm panels; the suggested sizes use d_max = 25 × the distance to the nearest
electrode, and the Mesh panel shows the slope.

The Mesh tab offers four finenesses (multiples of the suggested sizes); new projects
start at Fine. Measured on this trap with the compressed solver:

| fineness | scale | panels | H-matrix | solve (native) |
|---|---|---|---|---|
| Coarse | 1.4 | 19.5k | 0.66 GB | 0.9 s |
| Normal | 1 | 25k | 0.90 GB | 1.3 s |
| Fine (default) | 0.7 | 36.5k | 1.46 GB | 2.4 s |
| Very fine | 0.5 | 59k | 2.69 GB | 4.1 s |

Very fine is near the 4 GB limit of 32-bit WebAssembly; the 64-bit engine (below) has 16 GB.
The GUI warns when a mesh may not fit the running engine.

## WebAssembly

gmsh 4.15.2 and OpenCASCADE 7.9.3 compile to WebAssembly with Emscripten, so STEP import
and graded meshing can run in the browser; `scripts/build-wasm-engine.sh` links them with
the whole Rust application core into the browser engine (above). `scripts/build-wasm-gmsh.sh` builds everything
into `wasm-build/` (git-ignored; ~6 min) together with `wasm/mesh_step.c`, a C port of the
essentials of `mesh_bem` (curvature caps, distance-graded size field), as a Node program
and a browser module (`node wasm/serve.mjs wasm-build/out 8790`, then open the page).

Native vs browser on the trap (M5 Max, Chrome 157; `tf-bench` runs the same Rust solver
natively and as WebAssembly; WASM threads = 16):

| step | native app | Chrome WASM, 1 thread | Chrome WASM, threads |
|---|---|---|---|
| STEP import | 0.11 s | 0.41 s | (single-threaded) |
| display tessellation (85k tri) | 0.11 s | 3.1 s | (single-threaded) |
| graded BEM mesh | 2.3 s* | 1.3 s | (single-threaded) |
| BEM, dense LU, 15,594 panels | 4.4 s | 155 s | 14.4 s |
| BEM, compressed, 15,594 panels | 0.75 s | 5.8 s | 0.84 s |
| BEM, compressed, 17,382 panels | 1.26 s | — | 1.36 s |
| trap analysis | 0.6 ms | 1.0 ms | 1.0 ms |
| depth estimate / slice | 0.6 / 0.5 s | 10 / 7.9 s | 0.78 / 0.73 s |

\* includes the defect check and repair loop, which the C port skips. Dense needs one
2-GiB-max allocation in 32-bit WebAssembly (≤ 16,384 panels); the compressed solver does not.

### 64-bit memory (Memory64)

32-bit WebAssembly addresses at most 4 GB, which caps the browser at about 45k panels
(compressed) or 16k (dense). The engine is also built for `wasm64-unknown-emscripten`
(`-sMEMORY64`, up to 16 GB, the browsers' limit), and the page loads `tf_engine64.wasm`
where the browser validates a shared 64-bit memory, `tf_engine.wasm` elsewhere (or if the
64-bit one does not start); `?wasm=32` / `?wasm=64` force one. The status bar names the
running build. Building it:

```sh
WASM64=1 scripts/build-wasm-gmsh.sh      # gmsh + OpenCASCADE into wasm-build/*-mt64
WASM64=1 scripts/build-wasm-engine.sh    # -> frontend/public/engine/tf_engine64.{js,wasm}
```

wasm64 is a tier-3 Rust target: std is built from source (`-Z build-std`, as for the
threaded wasm32 build). Its exports take pointers and sizes as BigInt; the worker converts.
On the four-blade trap in Chrome, a dense solve of 19,492 panels (one 3.0 GB matrix, out
of reach of the 32-bit engine) ran in the browser and gave the native results; loading the
STEP file took about as long with either build.

Builds: `scripts/build-wasm-gmsh.sh` (gmsh + OpenCASCADE) and `scripts/build-wasm-solver.sh`
(threaded Rust solver: nightly Rust with a rebuilt std, Emscripten pthreads with
`PROXY_TO_PTHREAD`, mimalloc — Emscripten's default allocator serializes threads, which
made the H-matrix build 15× slower). `patches/atomic-wait` adds an Emscripten futex backend
so faer's thread pool runs on WebAssembly threads. Threads need cross-origin isolation
(COOP/COEP headers; `wasm/serve.mjs` sends them). Benchmark meshes:
`cargo run --release -p tf-geom --example export_meshes -- trap_electrodes_nothread.step
wasm-build/bench 1.4`; page: `wasm/bench.html`.

Notes: OpenCASCADE throws C++ exceptions; gmsh, OpenCASCADE and Rust must all use native
WebAssembly exceptions (`-fwasm-exceptions`, what rustc's Emscripten target uses).
OpenCASCADE is built without `OCC_CONVERT_SIGNALS`: its setjmp inside try blocks makes
LLVM emit invalid WebAssembly. gmsh meshing stays single-threaded in the browser (it
parallelizes with OpenMP, which Emscripten lacks).

## Tests

```sh
cargo test                # unit tests + integration test on the STEP files in the repo root
(cd frontend && npm run typecheck)
```

CI (`.github/workflows/ci.yml`) runs `cargo test --workspace` on Linux and macOS with the
gmsh SDK, and the frontend typecheck and build, on every push to main and every pull
request. The STEP integration tests skip there: the CAD files are not in the repository.
