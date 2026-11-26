import { create } from 'zustand';
import { connect } from './backend';
import { api, type AppStateDto, type ChainResult, type SliceResult, type SolveSummary, type SweepRow, type TrapParams, type TrapResult, type Vec3 } from './api';
import { decodeMesh, decodeSigma, decodeTess, type Sigma, type SurfMesh, type Tess } from './binary';
import { sliceScale, sweepMeta } from './results';
import type { Units } from './units';
import type { Hit } from './viewer/Viewer';

export interface Measurement {
  id: number;
  a: Vec3;
  b: Vec3;
}

export type SectionAxis = 'x' | 'y' | 'z' | null;
export type Tab = 'geometry' | 'electrodes' | 'mesh' | 'solve' | 'trap' | 'study';
/** what the workspace right of the sidebar shows */
export type Layout = '3d' | 'split' | 'results';
/** tabs with a results area (figures and tables) */
export const RESULT_TABS: Tab[] = ['solve', 'trap', 'study'];
/** tabs that look at the trap region rather than the whole model */
export const TRAP_TABS: Tab[] = ['trap', 'study'];

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
  showIons: boolean;
  /** clip the model to a box around the trap center */
  region: boolean;
  /** half-size of that box (m); null: twice the distance from the center to the nearest electrode */
  regionHalf: number | null;
}

export interface SweepState {
  rows: SweepRow[];
  /** axis label of the swept parameter, and its display unit and scale (SI / scale) */
  label: string;
  unit: string;
  scale: number;
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
  /** equilibrium positions of the last ion-chain calculation (m) */
  ions: Vec3[] | null;
  /** drive used by the Trap tab (also for sweeps and chains) */
  trapInput: TrapParams | null;
  /** solution version trapInput was set up for */
  trapInputFor: number | null;
  sweep: SweepState | null;
  chain: ChainResult | null;
  /** what the Study tab shows */
  studyMode: 'convergence' | 'tolerance';
  /** tolerance budget: standard deviation of every translation (m) and rotation (rad) component */
  tolSigma: { translate: number; rotate: number };
  layout: Record<Tab, Layout>;
  /** share of the workspace width given to the 3D view in the split layout */
  splitFrac: number;
  /** bumped to ask the viewer to frame the trap region */
  focusRequest: number;
  busy: string | null;
  error: string | null;
  /** an in-page question (e.g. before loading a very large file) */
  ask: Ask | null;
  tab: Tab;
  units: Units;
  /** 'features': clicks pick faces to remove (Geometry → Remove features) */
  tool: 'select' | 'measure' | 'features';
  /** the body last clicked: the one shown in detail and the anchor of shift-click ranges */
  selected: number | null;
  /** all selected bodies (includes `selected`) */
  selection: number[];
  hidden: number[];
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

  /** which backend runs the app (native server or in-browser engine) */
  backendLabel: string | null;
  backendKind: 'server' | 'browser' | null;
  /** most memory the engine can use (bytes); null: no limit beyond the machine's */
  memoryLimit: number | null;
  set: (p: Partial<Store>) => void;
  init: () => Promise<void>;
  setView: (p: Partial<ViewOptions>) => void;
  refresh: () => Promise<void>;
  apply: (dto: AppStateDto) => Promise<void>;
  run: (label: string, f: () => Promise<AppStateDto | void>) => Promise<void>;
  /** stop the running long job */
  stop: () => Promise<void>;
  /** upload and load a STEP file, asking first when it is very large */
  uploadStep: (f: File) => void;
}

export interface Ask {
  title: string;
  message: string;
  yes: string;
  onYes: () => void;
}

/** STEP files above this size get a warning before loading: OpenCASCADE's import and the
 * display tessellation of a file this size can take minutes (more in the browser). */
export const LARGE_STEP_BYTES = 10e6;

/** What the user should hear about a load that ended without a model. */
function loadOutcome(prev: AppStateDto | null, dto: AppStateDto): string | null {
  if (prev?.job.state !== 'running' || dto.job.kind !== 'load') return null;
  if (dto.job.state === 'failed') return `Loading failed: ${dto.job.error ?? 'unknown error'}`;
  if (dto.job.state === 'stopped') return dto.model ? null : 'Loading stopped: no model is loaded.';
  return null;
}

/**
 * What the pointer is over in the 3D view. Separate from the main store: it changes at
 * pointer rate, and only the status bar needs it.
 */
export const useHover = create<{ hover: Hit | null }>(() => ({ hover: null }));

const sameHit = (a: Hit | null, b: Hit | null) =>
  a === b || (!!a && !!b && a.body === b.body && a.face === b.face && a.snap === b.snap && a.point.every((x, i) => x === b.point[i]));

export function setHover(h: Hit | null) {
  if (!sameHit(useHover.getState().hover, h)) useHover.setState({ hover: h });
}

/**
 * Click on body `i` (null: on nothing). 'toggle' adds or removes it, 'range' selects every
 * body between the last clicked one and `i` (table order).
 */
export function selectBody(i: number | null, mode: 'one' | 'toggle' | 'range' = 'one') {
  const { selected, selection, set } = useStore.getState();
  if (i === null) {
    if (mode === 'one') set({ selected: null, selection: [] });
  } else if (mode === 'toggle') {
    if (!selection.includes(i)) set({ selected: i, selection: [...selection, i] });
    else {
      const rest = selection.filter((x) => x !== i);
      set({ selected: selected === i ? (rest.at(-1) ?? null) : selected, selection: rest });
    }
  } else if (mode === 'range' && selected !== null) {
    const [a, b] = selected < i ? [selected, i] : [i, selected];
    // the anchor stays put so further shift-clicks re-span from it
    set({ selection: Array.from({ length: b - a + 1 }, (_, k) => a + k) });
  } else set({ selected: i, selection: [i] });
}

let initStarted = false;
/** Order of state requests: a response older than the newest applied state is dropped. */
let stateSeq = 0;
/** solution version of the charge densities in the store */
let sigmaFor = -1;
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
  ions: null,
  trapInput: null,
  trapInputFor: null,
  sweep: null,
  chain: null,
  studyMode: 'convergence',
  tolSigma: { translate: 5e-6, rotate: 0.5e-3 },
  layout: { geometry: '3d', electrodes: '3d', mesh: '3d', solve: 'split', trap: 'split', study: 'results' },
  splitFrac: 0.42,
  focusRequest: 0,
  busy: null,
  error: null,
  ask: null,
  tab: 'geometry',
  units: 'mm',
  tool: 'select',
  selected: null,
  selection: [],
  hidden: [],
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
    showIons: true,
    region: false,
    regionHalf: null,
  },
  meshSizeRange: null,
  sigmaClip: null,
  fitRequest: 0,

  backendLabel: null,
  backendKind: null,
  memoryLimit: null,
  set: (p) => set(p),

  init: async () => {
    // once per page: React's development mode mounts twice, which would start two engines
    if (initStarted) return;
    initStarted = true;
    set({ busy: 'Connecting' });
    try {
      // (once connected, the end of a wait is not a reason to show "Connecting" again)
      const b = await connect((message) => set({ busy: message ?? (get().backendKind ? null : 'Connecting') }));
      set({ backendLabel: b.label, backendKind: b.kind, memoryLimit: b.memoryLimit });
      await get().refresh();
    } catch (e) {
      set({ error: `Could not start: ${(e as Error).message}` });
    } finally {
      set({ busy: null });
    }
  },
  setView: (p) => set({ view: { ...get().view, ...p } }),

  refresh: async () => {
    const id = ++stateSeq;
    try {
      const dto = await api.state();
      if (id !== stateSeq) return; // a newer request or command reply was applied meanwhile
      await get().apply(dto);
    } catch (e) {
      // a stop that the native server forces by restarting itself makes it briefly unreachable
      if (!get().app?.job.stopping) set({ error: (e as Error).message });
      // keep following a running job through transient failures
      if (get().app?.job.state === 'running') {
        window.clearTimeout(pollTimer);
        pollTimer = window.setTimeout(() => get().refresh(), 1000);
      }
    }
  },

  apply: async (dto) => {
    ++stateSeq;
    const prev = get().app;
    set({ app: dto });
    const told = (dto.notice !== prev?.notice && dto.notice) || loadOutcome(prev, dto);
    if (told) set({ error: told });
    window.clearTimeout(pollTimer);
    // poll faster during a solve so the convergence plot updates smoothly
    if (dto.job.state === 'running') pollTimer = window.setTimeout(() => get().refresh(), dto.job.kind === 'solve' || dto.job.kind === 'study' ? 200 : 400);

    // moved electrodes are drawn where they are: their displacements are part of the key
    const moves = JSON.stringify(dto.project.moves ?? {});
    const tk = dto.tess ? `${dto.tess.version}|${JSON.stringify(dto.frame)}|${moves}` : '';
    if (tk !== tessKey) {
      const newModel = !prev?.tess || prev.tess.version !== dto.tess?.version;
      const sameFrame = JSON.stringify(prev?.frame) === JSON.stringify(dto.frame);
      tessKey = tk;
      // geometry in a new frame: old picks and measurements no longer apply (an electrode
      // displacement keeps them)
      if (newModel || !sameFrame) set({ picked: null, pending: null, measurements: [] });
      useHover.setState({ hover: null });
      if (newModel) set({ selected: null, selection: [], hidden: [] });
      try {
        const tess = dto.tess ? decodeTess(await api.tess()) : null;
        if (tk === tessKey) set({ tess, fitRequest: newModel ? get().fitRequest + 1 : get().fitRequest });
      } catch (e) {
        if (tk === tessKey) tessKey = ''; // retry with the next state
        throw e;
      }
    }
    const mk = dto.mesh ? `${dto.mesh.version}|${moves}` : '';
    if (mk !== meshKey) {
      // only displaced electrodes: the same mesh, drawn where they now are
      const sameMesh = !!dto.mesh && meshKey.split('|')[0] === `${dto.mesh.version}`;
      meshKey = mk;
      let mesh;
      try {
        mesh = dto.mesh ? decodeMesh(await api.meshData()) : null;
      } catch (e) {
        if (mk === meshKey) meshKey = '';
        throw e;
      }
      if (mk === meshKey) {
        const v = get().view;
        if (sameMesh) set({ mesh });
        else set({ mesh, view: { ...v, showMesh: !!mesh, meshColor: v.meshColor === 'sigma' ? 'size' : v.meshColor } });
      }
    }
    // solution summary: refetch on a new solution or when its staleness flips
    const sk = dto.solve ? `${dto.solve.version}|${dto.solve.stale}` : '';
    if (sk !== solveKey) {
      const newVersion = !dto.solve || solveKey.split('|')[0] !== `${dto.solve.version}`;
      solveKey = sk;
      set({ trap: null, slice: null, ions: null, sweep: null, chain: null });
      if (!dto.solve) {
        set({ solution: null, sigma: null });
        if (get().view.meshColor === 'sigma') set({ view: { ...get().view, meshColor: 'size' } });
      } else {
        const version = dto.solve.version;
        let fetched;
        try {
          fetched = await Promise.all([
            api.solution(),
            newVersion || sigmaFor !== version ? api.sigma().then(decodeSigma) : Promise.resolve(get().sigma),
            api.results().catch(() => null),
          ]);
        } catch (e) {
          if (sk === solveKey) solveKey = '';
          throw e;
        }
        const [solution, sigma, saved] = fetched;
        if (sk === solveKey) {
          sigmaFor = version;
          set({ solution, sigma });
          // results saved with this solution (sweeps, chains, slices) come back with it
          if (saved) {
            set({
              sweep: saved.sweep ? { rows: saved.sweep.result.rows, ...sweepMeta(saved.sweep.args.variable) } : null,
              chain: saved.chain?.result ?? null,
              ions: saved.chain?.result.positions ?? null,
              slice: saved.slice ? { data: saved.slice.result, ...sliceScale(saved.slice.result) } : null,
            });
          }
        }
      }
    }
  },

  stop: async () => {
    try {
      await api.cancel();
    } catch (e) {
      set({ error: (e as Error).message });
    }
    await get().refresh();
  },

  uploadStep: (f) => {
    const go = () => get().run(`Uploading ${f.name}`, () => api.upload(f));
    if (f.size < LARGE_STEP_BYTES) return void go();
    const mb = (f.size / 1e6).toFixed(0);
    set({
      ask: {
        title: `${f.name} is ${mb} MB`,
        message:
          `Importing a STEP file this large and tessellating it for display can take several minutes${get().backendKind === 'browser' ? ', longer in the browser than with the native server' : ''}. ` +
          'You can stop the load from the top bar at any time.',
        yes: 'Load it',
        onYes: go,
      },
    });
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
