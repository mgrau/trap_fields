# trapfields

Fields, potentials and pseudopotentials of realistic RF Paul traps, starting from a CAD
(STEP) model of the electrodes.

Status: load a STEP export, inspect and measure it, orient it into the simulation frame,
label electrodes, generate a graded surface mesh, solve for one basis potential per
electrode (with its solid-harmonic expansion about the trap center), and analyze the
trap for any RF/DC drive: pseudopotential, equilibrium, secular modes, Mathieu
parameters, Floquet stability, anharmonicity, depth, and slices in the 3D view.

The mesh and solution are saved in the project folder and restored when the GUI is
restarted (if the STEP file and transforms are unchanged).

## Layout

```
crates/tf-gmsh    safe Rust wrapper around the gmsh C API (gmsh + OpenCASCADE do STEP import and meshing)
crates/tf-geom    STEP name/unit recovery, rigid transforms, display tessellation, graded BEM mesh
crates/tf-bem     BEM electrostatics: panel integrals, dense LU solve, solid-harmonic expansions
crates/tf-server  `trapfields` binary: local HTTP API + serves the GUI
frontend/         TypeScript + React + three.js GUI
scripts/          fetch-gmsh.sh (downloads the gmsh SDK into third_party/)
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
(selected file, transforms, electrode labels, mesh settings) and `mesh/` there.
The STEP file itself is never modified.

Frontend development with hot reload: run the server as above, then
`cd frontend && npm run dev` and open http://localhost:5173 (API calls are proxied).

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
trap depth (lowest barrier along 400 straight rays from the ion; the true saddle can be
lower) and energy slices through the ion, drawn in the 3D view with contours.

`POST /api/trap` and `POST /api/trap/slice` expose the same analysis.

## WebAssembly (experimental)

gmsh 4.15.2 and OpenCASCADE 7.9.3 compile to WebAssembly with Emscripten, so STEP import
and graded meshing can run in the browser. `scripts/build-wasm-gmsh.sh` builds everything
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

Builds: `scripts/build-wasm-gmsh.sh` (gmsh + OpenCASCADE) and `scripts/build-wasm-solver.sh`
(threaded Rust solver: nightly Rust with a rebuilt std, Emscripten pthreads with
`PROXY_TO_PTHREAD`, mimalloc — Emscripten's default allocator serializes threads, which
made the H-matrix build 15× slower). `patches/atomic-wait` adds an Emscripten futex backend
so faer's thread pool runs on WebAssembly threads. Threads need cross-origin isolation
(COOP/COEP headers; `wasm/serve.mjs` sends them). Benchmark meshes:
`cargo run --release -p tf-geom --example export_meshes -- trap_electrodes_nothread.step
wasm-build/bench 1.4`; page: `wasm/bench.html`.

Notes: OpenCASCADE throws C++ exceptions, so gmsh must be compiled with the same
`-fexceptions` mode. gmsh meshing stays single-threaded in the browser (it parallelizes
with OpenMP, which Emscripten lacks).

## Tests

```sh
cargo test                # unit tests + integration test on the STEP files in the repo root
(cd frontend && npm run typecheck)
```
