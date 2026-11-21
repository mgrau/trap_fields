// Typed client for the application core, over whichever backend is connected (native
// server or in-browser engine, see backend.ts). All lengths are SI meters.
import { connected } from './backend';
import { track } from './progress';

export type Vec3 = [number, number, number];
export type Role = 'unassigned' | 'rf' | 'dc' | 'ground' | 'floating';
export const ROLES: Role[] = ['unassigned', 'rf', 'dc', 'ground', 'floating'];

export interface Label {
  electrode: string;
  role: Role;
}

export type TransformOp =
  | { op: 'rotate'; axis: Vec3; angle_deg: number; center?: Vec3 }
  | { op: 'translate'; delta: Vec3 };

export interface MeshParams {
  center: Vec3;
  h_min: number;
  h_max: number;
  dist_min: number;
  dist_max: number;
  min_circle_points: number;
}

export interface MeshStats {
  n_nodes: number;
  n_triangles: number;
  triangles_per_body: number[];
  edge_min: number;
  edge_max: number;
  edge_mean: number;
  area_total: number;
  failed_faces: number[];
  defective_faces: number[];
  curvature_capped_faces: number;
  repair_rounds: number;
  seconds: number;
}

export type SolverChoice = 'auto' | 'dense' | 'compressed';

export interface SolverSettings {
  aca_eps: number;
  gmres_tol: number;
  eta: number;
  leaf_size: number;
}

export const DEFAULT_SOLVER_SETTINGS: SolverSettings = { aca_eps: 1e-6, gmres_tol: 1e-9, eta: 2, leaf_size: 64 };

export interface SolveParams {
  lmax: number;
  center: Vec3 | null;
  r0: number | null;
  method?: SolverChoice;
  settings?: SolverSettings;
}

export interface Local {
  phi: number;
  grad: Vec3;
  hessian: number[][];
}

export interface BasisResult {
  electrode: string;
  index: number;
  /** charge (C) on each electrode, in the order of `electrodes` */
  charge: number[];
  /** potential (V) of each electrode (floating ones solved for) */
  potential: number[];
  /** a_lm (V) at flat index l*l + l + m */
  coeffs: number[];
  local: Local;
  sigma_range: [number, number] | null;
}

export interface SolveSummary {
  version: number;
  mesh_version: number;
  n_panels: number;
  n_unknowns: number;
  matrix_bytes: number;
  timings: { assemble: number; factor: number; solve: number };
  solver: {
    method: 'dense' | 'compressed';
    matrix_bytes: number;
    compression: number | null;
    max_rank: number | null;
    iterations: number[];
    residual: number[];
    history?: number[][];
  };
  expand_seconds: number;
  seconds: number;
  params: SolveParams;
  center: Vec3;
  r0: number;
  r_min: number;
  lmax: number;
  electrodes: { name: string; role: Role; floating: boolean; bodies: string[] }[];
  basis: BasisResult[];
  stale: boolean;
}

/** n[a] nodes along axis a from center − half to center + half (one node: the center) */
export interface FieldGrid {
  center: Vec3;
  half: Vec3;
  n: [number, number, number];
}

/** most grid points per field export (tf-app export.rs) */
export const MAX_EXPORT_POINTS = 125_000;

export interface TrapParams {
  ion: { mass_amu: number; charge_e: number };
  /** Ω/2π (Hz) */
  rf_freq: number;
  /** RF amplitude (V, zero to peak) per electrode */
  rf: Record<string, number>;
  /** static voltage per electrode */
  dc: Record<string, number>;
  depth: boolean;
}

export interface Mode {
  axis: Vec3;
  curvature: number;
  curvature_rf: number;
  freq_pseudo: number | null;
  freq_floquet: number | null;
  c2: number;
  c3: number;
  c4: number;
}

export interface TrapResult {
  analysis: {
    rf_null: Vec3;
    rf_null_residual: number;
    rf_null_line: Vec3 | null;
    equilibrium: Vec3;
    confined: boolean;
    rf_field_at_eq: Vec3;
    energy_eq: number;
    modes: Mode[];
    mathieu: { axis: Vec3; a: number; q: number }[];
    stable: boolean | null;
    notes: string[];
  };
  depth: {
    depth: number;
    escape_direction: Vec3;
    barrier_point: Vec3;
    /** saddle between the electrodes, or the route climbs until an electrode / the search region's edge */
    exit: 'saddle' | 'electrode' | 'boundary';
    /** the saddle was located exactly, not only to the search grid */
    refined: boolean;
    /** the escape route from the ion through the barrier (m) */
    path: Vec3[];
    grid: number;
    r_far: number;
  } | null;
  depth_ev: number | null;
  energy_eq_ev: number;
  r_valid: number;
  center: Vec3;
  seconds: number;
}

export interface SliceResult {
  axis: number;
  u_axis: number;
  v_axis: number;
  center: Vec3;
  half: number;
  n: number;
  /** U (eV) row-major [v][u]; null inside electrodes */
  values: (number | null)[];
  /** within this distance of the expansion center (m), U comes from the expansion */
  r_expansion?: number;
}

export type SweepVar = { kind: 'rf_amplitude' } | { kind: 'rf_freq' } | { kind: 'dc'; electrode: string };

export interface SweepRow {
  value: number;
  confined?: boolean;
  stable?: boolean | null;
  freq_pseudo?: (number | null)[];
  freq_floquet?: (number | null)[];
  axes?: Vec3[];
  q?: number[];
  a?: number[];
  equilibrium?: Vec3;
  error?: string;
}

export interface ChainMode {
  freq: number;
  vector: Vec3[];
  axis_weights: [number, number, number];
}

export interface ChainResult {
  n: number;
  positions: Vec3[];
  chain_axis: Vec3;
  principal_axes: [Vec3, Vec3, Vec3];
  min_spacing: number;
  transverse: number;
  modes: ChainMode[];
  stable: boolean;
  iterations: number;
  seconds: number;
}

export interface StudyLevel {
  scale: number;
  n_panels: number;
  mesh_seconds: number;
  solve_seconds: number;
  method: string;
  iterations: number | null;
  trap_confined: boolean | null;
  values: { name: string; unit: string; value: number }[];
}

export interface StudyQuantity {
  name: string;
  unit: string;
  values: (number | null)[];
  extrapolated?: number;
  order?: number;
  error_estimate?: number;
}

export interface StudyResult {
  levels: StudyLevel[];
  quantities: StudyQuantity[];
  base: MeshParams;
  seconds: number;
  /** false when the study was stopped before its last level */
  complete?: boolean;
}

export interface Saved<A, R> {
  args: A;
  result: R;
}

export interface SavedResults {
  sweep: Saved<{ params: TrapParams; variable: SweepVar; values: number[] }, { rows: SweepRow[] }> | null;
  chain: Saved<{ params: TrapParams; n: number }, ChainResult> | null;
  slice: Saved<{ params: TrapParams; axis: number; center: Vec3; half: number; n: number }, SliceResult> | null;
}

export interface EvalResult {
  phi: number[];
  grad: Vec3[];
  expansion: ({ phi: number; grad: Vec3 } | null)[] | null;
}

export interface BodyState {
  index: number;
  name: string;
  step_name: string;
  n_faces: number;
  volume: number;
  area: number;
  name_match_cost: number | null;
  bbox: number[] | null;
  closest: { distance: number; point: Vec3 } | null;
  label: Label | null;
}

export interface Job {
  state: 'idle' | 'running' | 'done' | 'failed' | 'stopped';
  kind?: string;
  /** a stop was requested and the job has not reached a checkpoint yet */
  stopping?: boolean;
  elapsed?: number;
  stage?: string;
  seconds?: number;
  error?: string;
}

/** A face by its geometry in the STEP frame (identifies it across reloads). */
export interface FaceRef {
  center: Vec3;
  area: number;
}

/** One defeaturing operation: faces removed from one body. */
export interface FeatureRemoval {
  body: string;
  faces: FaceRef[];
}

/** Faces picked for removal: tag (as in the tessellation), surface type and area (m²). */
export interface FeatureFace {
  tag: number;
  kind: string;
  area: number;
}

export interface AppStateDto {
  project_dir: string;
  files: string[];
  project: {
    step_file: string | null;
    transforms: TransformOp[];
    /** absent when empty */
    removed_features?: FeatureRemoval[];
    labels: Record<string, Label>;
    mesh_params: MeshParams | null;
    solve_params: SolveParams | null;
    trap_params: TrapParams | null;
  };
  model: {
    file_name: string;
    length_unit: string;
    products: string[];
    originating_system: string;
    n_faces: number;
    n_curves: number;
    n_points: number;
    warnings: string[];
    load_seconds: number;
  } | null;
  bodies: BodyState[];
  bbox: number[] | null;
  frame: { r: number[][]; t: Vec3 };
  tess: { version: number; n_triangles: number; failed_faces: number[]; seconds: number } | null;
  mesh: { version: number; params: MeshParams; stats: MeshStats } | null;
  mesh_defaults: MeshParams | null;
  solve: { version: number; mesh_version: number; n_panels: number; seconds: number; stale: boolean } | null;
  job: Job;
  /** live progress of the running job */
  progress?: { gmres?: [number, number[]][]; study?: StudyLevel[]; step?: [number, number] | null };
  study?: StudyResult | null;
  notes?: string[];
  uploaded_as?: string;
  /** something to tell the user (e.g. the engine restarted to stop a job); stays until
   * the next long job, and is shown when it changes */
  notice?: string;
}

const b = () => connected;
const n3 = (g: FieldGrid) => g.n[0] * g.n[1] * g.n[2];

export const api = {
  state: () => b().state(),
  open: (file: string) => b().cmd<AppStateDto>('open', { file }),
  upload: async (file: File): Promise<AppStateDto> => b().cmd<AppStateDto>('upload', { name: file.name }, await file.arrayBuffer()),
  /** sweep, chain and slice results saved for the current solution */
  results: () => track('results', 'Loading results', 1, () => b().cmd<SavedResults>('results')),
  /** the project (file, STEP, mesh, solution, results) as a tar archive */
  exportProject: () => track('export_project', 'Packing the project', 1, () => b().bin('export_project')),
  /** replace the project with a bundle from exportProject (long job) */
  importProject: async (file: File): Promise<AppStateDto> => b().cmd<AppStateDto>('import_project', null, await file.arrayBuffer()),
  transform: (op: TransformOp) => track('transform', 'Transforming', 1, () => b().cmd<AppStateDto>('transform', op)),
  undo: () => track('transform', 'Transforming', 1, () => b().cmd<AppStateDto>('transform_undo', null)),
  resetTransforms: () => track('transform', 'Transforming', 1, () => b().cmd<AppStateDto>('transform_reset', null)),
  /** the face and, with grow, the rest of the feature (hole, thread) it belongs to */
  featureFaces: (face: number, grow: boolean) =>
    track('feature_faces', 'Finding the feature', 1, () => b().cmd<{ body: number; faces: FeatureFace[] }>('feature_faces', { face, grow })),
  /** remove faces of one body (long job) */
  removeFeatures: (faces: number[]) => b().cmd<AppStateDto>('remove_features', { faces }),
  undoFeatures: () => b().cmd<AppStateDto>('features_undo', null),
  resetFeatures: () => b().cmd<AppStateDto>('features_reset', null),
  labels: (labels: Record<string, Label>) => track('labels', 'Saving electrodes', 1, () => b().cmd<AppStateDto>('labels', labels)),
  mesh: (params: MeshParams) => b().cmd<AppStateDto>('mesh', params),
  tess: () => track('tess', 'Loading the display model', 1, () => b().bin('tess')),
  meshData: () => track('mesh_data', 'Loading the mesh', 1, () => b().bin('mesh_data')),
  solve: (params: SolveParams) => b().cmd<AppStateDto>('solve', params),
  solution: () => track('solution', 'Loading the solution', 1, () => b().cmd<SolveSummary>('solution')),
  sigma: () => track('sigma', 'Loading charge densities', 1, () => b().bin('sigma')),
  trap: (p: TrapParams) =>
    track(p.depth ? 'trap_depth' : 'trap', p.depth ? 'Trap depth' : 'Trap analysis', 1, () => b().cmd<TrapResult>('trap', p)),
  slice: (params: TrapParams, axis: number, center: Vec3, half: number, n: number) =>
    track('slice', 'Slice', n * n, () => b().cmd<SliceResult>('slice', { params, axis, center, half, n })),
  evaluate: (points: Vec3[], voltages: Record<string, number>) =>
    track('eval', 'Evaluating', points.length, () => b().cmd<EvalResult>('eval', { points, voltages })),
  /** φ and E on a regular grid as CSV, per electrode basis solution or for one drive */
  exportFields: (grid: FieldGrid, fields: { kind: 'basis' } | { kind: 'drive'; params: TrapParams }) =>
    track(`export_${fields.kind}`, 'Exporting fields', n3(grid), () => b().cmd<ArrayBuffer>('export_fields', { grid, fields })),
  sweep: (params: TrapParams, variable: SweepVar, values: number[]) =>
    track('sweep', 'Sweep', values.length, () => b().cmd<{ rows: SweepRow[]; seconds: number }>('sweep', { params, variable, values })),
  chain: (params: TrapParams, n: number) => track('chain', 'Ion chain', n ** 3, () => b().cmd<ChainResult>('chain', { params, n })),
  /** stop the running long job at its next checkpoint */
  cancel: () => b().cancel(),
  study: (args: { base: MeshParams; scales: number[]; solve: SolveParams; trap: TrapParams | null }) => b().cmd<AppStateDto>('study', args),
};
