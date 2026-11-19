import { create } from 'zustand';
import { api, type AppStateDto, type SliceResult, type SolveSummary, type TrapResult, type Vec3 } from './api';
import { decodeMesh, decodeSigma, decodeTess, type Sigma, type SurfMesh, type Tess } from './binary';
import type { Units } from './units';
import type { Hit } from './viewer/Viewer';

export interface Measurement {
  id: number;
  a: Vec3;
  b: Vec3;
}

export type SectionAxis = 'x' | 'y' | 'z' | null;
export type Tab = 'geometry' | 'electrodes' | 'mesh' | 'solve' | 'trap';

export interface ViewOptions {
  projection: 'persp' | 'ortho';
  rotate: 'orbit' | 'free';
  edges: boolean;
  xray: boolean;
  axes: boolean;
  colorBy: 'electrode' | 'role';
  section: { axis: SectionAxis; offset: number; flip: boolean };
  showMesh: boolean;
  meshColor: 'size' | 'body' | 'sigma';
  /** basis solution shown when meshColor is 'sigma' */
  sigmaBasis: number;
  showModes: boolean;
  showSlice: boolean;
}

export interface SliceState {
  data: SliceResult;
  /** energies (eV) above U_eq + clip are drawn translucent */
  clip: number;
  /** U at the equilibrium (eV) */
  ref: number;
}

interface Store {
  app: AppStateDto | null;
  tess: Tess | null;
  mesh: SurfMesh | null;
  solution: SolveSummary | null;
  sigma: Sigma | null;
  trap: TrapResult | null;
  slice: SliceState | null;
  busy: string | null;
  error: string | null;
  tab: Tab;
  units: Units;
  tool: 'select' | 'measure';
  selected: number | null;
  hidden: number[];
  hover: Hit | null;
  picked: Hit | null;
  pending: Vec3 | null;
  measurements: Measurement[];
  view: ViewOptions;
  /** panel-size range (m) of the displayed BEM mesh, for the legend */
  meshSizeRange: [number, number] | null;
  /** |σ| (C/m²) at which the charge-density colors saturate */
  sigmaClip: number | null;
  /** bumped to ask the viewer to re-fit the camera */
  fitRequest: number;

  set: (p: Partial<Store>) => void;
  setView: (p: Partial<ViewOptions>) => void;
  refresh: () => Promise<void>;
  apply: (dto: AppStateDto) => Promise<void>;
  run: (label: string, f: () => Promise<AppStateDto | void>) => Promise<void>;
}

let tessKey = '';
let meshKey = '';
let solveKey = '';
let pollTimer: number | undefined;
let nextMeasureId = 1;
export const newMeasureId = () => nextMeasureId++;

export const useStore = create<Store>((set, get) => ({
  app: null,
  tess: null,
  mesh: null,
  solution: null,
  sigma: null,
  trap: null,
  slice: null,
  busy: null,
  error: null,
  tab: 'geometry',
  units: 'mm',
  tool: 'select',
  selected: null,
  hidden: [],
  hover: null,
  picked: null,
  pending: null,
  measurements: [],
  view: {
    projection: 'persp',
    rotate: 'orbit',
    edges: true,
    xray: false,
    axes: true,
    colorBy: 'electrode',
    section: { axis: null, offset: 0, flip: false },
    showMesh: false,
    meshColor: 'size',
    sigmaBasis: 0,
    showModes: true,
    showSlice: true,
  },
  meshSizeRange: null,
  sigmaClip: null,
  fitRequest: 0,

  set: (p) => set(p),
  setView: (p) => set({ view: { ...get().view, ...p } }),

  refresh: async () => {
    try {
      await get().apply(await api.state());
    } catch (e) {
      set({ error: (e as Error).message });
    }
  },

  apply: async (dto) => {
    const prev = get().app;
    set({ app: dto });
    window.clearTimeout(pollTimer);
    if (dto.job.state === 'running') pollTimer = window.setTimeout(() => get().refresh(), 400);

    const tk = dto.tess ? `${dto.tess.version}|${JSON.stringify(dto.frame)}` : '';
    if (tk !== tessKey) {
      const newModel = !prev?.tess || prev.tess.version !== dto.tess?.version;
      tessKey = tk;
      // geometry in a new frame: old picks and measurements no longer apply
      set({ picked: null, pending: null, measurements: [], hover: null });
      if (newModel) set({ selected: null, hidden: [] });
      const tess = dto.tess ? decodeTess(await api.tess()) : null;
      if (tk === tessKey) set({ tess, fitRequest: newModel ? get().fitRequest + 1 : get().fitRequest });
    }
    const mk = dto.mesh ? `${dto.mesh.version}` : '';
    if (mk !== meshKey) {
      meshKey = mk;
      const mesh = dto.mesh ? decodeMesh(await api.meshData()) : null;
      if (mk === meshKey) {
        const v = get().view;
        set({ mesh, view: { ...v, showMesh: !!mesh, meshColor: v.meshColor === 'sigma' ? 'size' : v.meshColor } });
      }
    }
    // solution summary: refetch on a new solution or when its staleness flips
    const sk = dto.solve ? `${dto.solve.version}|${dto.solve.stale}` : '';
    if (sk !== solveKey) {
      const newVersion = !dto.solve || solveKey.split('|')[0] !== `${dto.solve.version}`;
      solveKey = sk;
      set({ trap: null, slice: null });
      if (!dto.solve) {
        set({ solution: null, sigma: null });
        if (get().view.meshColor === 'sigma') set({ view: { ...get().view, meshColor: 'size' } });
      } else {
        const [solution, sigma] = await Promise.all([api.solution(), newVersion ? api.sigma().then(decodeSigma) : Promise.resolve(get().sigma)]);
        if (sk === solveKey) set({ solution, sigma });
      }
    }
  },

  run: async (label, f) => {
    set({ busy: label, error: null });
    try {
      const dto = await f();
      if (dto) await get().apply(dto);
    } catch (e) {
      set({ error: (e as Error).message });
      await get().refresh();
    } finally {
      set({ busy: null });
    }
  },
}));
