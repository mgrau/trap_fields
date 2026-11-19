// Typed client for the Rust backend (`/api`). All lengths are SI meters.

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

export interface SolveParams {
  lmax: number;
  center: Vec3 | null;
  r0: number | null;
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
  depth: { depth: number; escape_direction: Vec3; barrier_point: Vec3; n_rays: number } | null;
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
  state: 'idle' | 'running' | 'done' | 'failed';
  kind?: string;
  elapsed?: number;
  stage?: string;
  seconds?: number;
  error?: string;
}

export interface AppStateDto {
  project_dir: string;
  files: string[];
  project: {
    step_file: string | null;
    transforms: TransformOp[];
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
  uploaded_as?: string;
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    /* non-JSON error page */
  }
  if (!res.ok) {
    const msg = (data as { error?: string } | null)?.error ?? `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  return data as T;
}

async function binary(path: string): Promise<ArrayBuffer> {
  const res = await fetch(`/api${path}`);
  if (!res.ok) throw new Error(`${path}: ${res.status} ${res.statusText}`);
  return res.arrayBuffer();
}

export const api = {
  state: () => call<AppStateDto>('GET', '/state'),
  open: (file: string) => call<AppStateDto>('POST', '/open', { file }),
  upload: async (file: File): Promise<AppStateDto> => {
    const res = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, { method: 'POST', body: file });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error ?? `${res.status} ${res.statusText}`);
    return data as AppStateDto;
  },
  transform: (op: TransformOp) => call<AppStateDto>('POST', '/transform', op),
  undo: () => call<AppStateDto>('POST', '/transform/undo'),
  resetTransforms: () => call<AppStateDto>('POST', '/transform/reset'),
  labels: (labels: Record<string, Label>) => call<AppStateDto>('PUT', '/labels', labels),
  mesh: (params: MeshParams) => call<AppStateDto>('POST', '/mesh', params),
  tess: () => binary('/tess'),
  meshData: () => binary('/mesh'),
  solve: (params: SolveParams) => call<AppStateDto>('POST', '/solve', params),
  solution: () => call<SolveSummary>('GET', '/solve'),
  sigma: () => binary('/solve/sigma'),
  trap: (p: TrapParams) => call<TrapResult>('POST', '/trap', p),
  slice: (params: TrapParams, axis: number, center: Vec3, half: number, n: number) =>
    call<SliceResult>('POST', '/trap/slice', { params, axis, center, half, n }),
  evaluate: (points: Vec3[], voltages: Record<string, number>) => call<EvalResult>('POST', '/solve/eval', { points, voltages }),
};
