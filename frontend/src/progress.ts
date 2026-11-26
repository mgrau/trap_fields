// How far along the running work is, for the progress bar: one fraction from 0 to 1 over
// the whole job (not per stage), never going back. Approximate where it has to be:
//
// - a long job (load, mesh, solve, study, feature edits) runs through known stages, each
//   with a share of the bar; inside a stage the fraction comes from what the engine
//   counts (bodies, faces, GMRES residual) or, where it counts nothing (an LU
//   factorization, a gmsh pass), from the time the stage took last time, scaled by the
//   problem size;
// - a quick command (trap depth, slice, sweep, export, …) is timed against how long the
//   same command took per unit of work before.
//
// Timings are remembered per browser (localStorage), so estimates improve with use.
import { create } from 'zustand';
import { DEFAULT_SOLVER_SETTINGS, type AppStateDto } from './api';

// ------------------------------------------------------------------ learned timings

/** ms per unit of work, per key (a job stage or a command) */
type Timings = Record<string, number>;
const STORE_KEY = 'trapfields.timings';
let timings: Timings = (() => {
  try {
    return JSON.parse(localStorage.getItem(STORE_KEY) ?? '{}') as Timings;
  } catch {
    return {};
  }
})();

function learn(key: string, ms: number, work: number) {
  if (!(ms > 0 && work > 0)) return;
  const rate = ms / work;
  const old = timings[key];
  // smooth over runs, but follow a big change (another machine, another engine) quickly
  timings = { ...timings, [key]: old && old / rate < 4 && rate / old < 4 ? 0.5 * old + 0.5 * rate : rate };
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(timings));
  } catch {
    /* private window or storage blocked: estimates last for this page only */
  }
}

/** Expected duration (ms) of `work` units under `key`, or `fallback` ms if never timed. */
function expected(key: string, work: number, fallback: number): number {
  const r = timings[key];
  return r ? r * work : fallback;
}

/** Fraction of an operation expected to take `e` ms after `t` ms: about t/e early on,
 *  then slowing so an underestimate never reaches the end (at most 0.95). */
function eased(t: number, e: number): number {
  return Math.min(0.95, 1 - Math.exp((-1.2 * t) / Math.max(e, 1)));
}

// ------------------------------------------------------------------ long jobs

/** A stage of a job: its share of the bar, how its duration grows with the problem size
 *  n (∝ n^exp), and its duration in ms before it was ever timed. */
interface Stage {
  match: RegExp;
  weight: number;
  exp?: number;
  default?: (n: number) => number;
}

const LOAD: Stage[] = [
  { match: /^reading STEP/, weight: 2 },
  { match: /^importing STEP/, weight: 25 },
  { match: /^building the model/, weight: 3 },
  { match: /^matching body names/, weight: 2 },
  { match: /^measuring bodies/, weight: 8 },
  { match: /^removing features/, weight: 25 },
  { match: /^tessellating/, weight: 35 },
  { match: /^meshing the faces gmsh skipped/, weight: 5 },
  { match: /^collecting display triangles/, weight: 5 },
  { match: /^collecting edges/, weight: 4 },
  { match: /^restoring mesh and solution/, weight: 3 },
];

const SOLVE_DENSE: Stage[] = [
  { match: /^assembling/, weight: 15, exp: 2, default: (n) => 3e-5 * n * n },
  { match: /^factoring/, weight: 70, exp: 3, default: (n) => 3e-9 * n ** 3 },
  { match: /^solving/, weight: 5, exp: 2, default: (n) => 1e-6 * n * n },
  { match: /^expanding/, weight: 10, exp: 1, default: (n) => 0.02 * n },
];

const SOLVE_COMPRESSED: Stage[] = [
  { match: /^building H-matrix/, weight: 30, exp: 1.3, default: (n) => 0.1 * n },
  { match: /^preconditioner/, weight: 5, exp: 1, default: (n) => 0.01 * n },
  { match: /^GMRES/, weight: 55, exp: 1.3, default: (n) => 0.2 * n },
  { match: /^expanding/, weight: 10, exp: 1, default: (n) => 0.02 * n },
];

const PLANS: Record<string, Stage[]> = {
  load: [{ match: /^unpacking/, weight: 5 }, ...LOAD],
  remove_features: LOAD,
  features_undo: LOAD,
  features_reset: LOAD,
  mesh: [{ match: /^meshing/, weight: 100, default: () => 5000 }],
};

/** Stages of one level of a convergence study: meshing, then a solve. */
const studyLevel = (compressed: boolean): Stage[] => [
  { match: /^meshing/, weight: 30, default: () => 5000 },
  { match: /^solving \d+ panels/, weight: 1 },
  ...(compressed ? SOLVE_COMPRESSED : SOLVE_DENSE).map((s) => ({ ...s, weight: 0.69 * s.weight })),
];

/** Stages of one case of a tolerance study: moving one electrode, then a solve. */
const toleranceCase = (compressed: boolean): Stage[] => [{ match: /^moving/, weight: 1 }, ...(compressed ? SOLVE_COMPRESSED : SOLVE_DENSE)];

/**
 * GMRES: the relative residual falls from 1 to the tolerance roughly geometrically, so
 * progress is log(residual)/log(tolerance). All right-hand sides (one per electrode)
 * iterate together; the slowest one decides when the solve ends.
 */
function gmresFraction(gmres: [number, number[]][] | undefined, tol: number): number | null {
  const last = gmres?.[gmres.length - 1]?.[1];
  if (!last?.length || !(tol > 0 && tol < 1)) return null;
  const f = Math.min(...last.map((r) => (r > 0 ? Math.log(r) / Math.log(tol) : 1)));
  return Math.min(1, Math.max(0, f));
}

/** Follows one running job across polls: when each stage began, how far the bar got. */
interface Follow {
  job: string;
  stage: string;
  stageKey: string | null;
  stageStart: number;
  stageSize: number;
  index: number;
  best: number;
}
let follow: Follow | null = null;

/** Whether a solve of `n` panels with these settings runs the compressed solver. */
function isCompressed(app: AppStateDto, stage: string): boolean {
  if (/H-matrix|preconditioner|GMRES/.test(stage)) return true;
  if (/assembling|factoring|^solving$/.test(stage)) return false;
  const m = app.project.solve_params?.method ?? 'auto';
  return m === 'compressed' || (m === 'auto' && (app.mesh?.stats.n_triangles ?? 0) >= 5000);
}

/**
 * Fraction (0 to 0.99) of the running job at time `now` (ms, performance.now()), or null
 * if no job runs. Call it often (each frame): stages that count nothing advance with time.
 */
export function jobFraction(app: AppStateDto | null, now: number): number | null {
  const job = app?.job;
  if (!app || job?.state !== 'running') {
    // the job ended: its last stage ended with it (a finished one; a stopped or failed
    // job says nothing about how long the stage takes)
    if (follow?.stageKey && job?.state === 'done') learn(follow.stageKey, now - follow.stageStart, follow.stageSize);
    follow = null;
    return null;
  }
  const kind = JOB_KIND_ALIASES[job.kind ?? ''] ?? job.kind ?? '';
  const id = `${kind}|${app.project.step_file ?? ''}`;
  let stage = job.stage ?? '';
  let level: [number, number] | null = null;
  if (kind === 'study' || kind === 'tolerance') {
    // "level 2/4: GMRES", "case 3/73 (rod ++ +x): GMRES"
    const m = kind === 'study' ? /^level (\d+)\/(\d+): (.*)$/.exec(stage) : /^case (\d+)\/(\d+) \([^)]*\): (.*)$/.exec(stage);
    if (m) {
      level = [Number(m[1]) - 1, Number(m[2])];
      stage = m[3];
    }
  }
  const n = app.mesh?.stats.n_triangles ?? 0;
  const plan =
    kind === 'study'
      ? studyLevel(isCompressed(app, stage))
      : kind === 'tolerance'
        ? toleranceCase(isCompressed(app, stage))
        : kind === 'solve'
          ? isCompressed(app, stage)
            ? SOLVE_COMPRESSED
            : SOLVE_DENSE
          : (PLANS[kind] ?? []);
  const total = plan.reduce((a, s) => a + s.weight, 0) || 1;

  // a new job, or a new stage: time the one that ended
  const stageId = level ? `${level[0]}|${stage}` : stage;
  if (!follow || follow.job !== id) follow = { job: id, stage: '', stageKey: null, stageStart: now, stageSize: n, index: 0, best: 0 };
  if (follow.stage !== stageId) {
    if (follow.stageKey && follow.stage) learn(follow.stageKey, now - follow.stageStart, follow.stageSize);
    const i = plan.findIndex((s) => s.match.test(stage));
    // an unknown stage continues the one before it
    if (i >= 0) follow.index = i;
    const st = plan[follow.index];
    follow.stage = stageId;
    follow.stageKey = i >= 0 && st ? `stage|${kind === 'study' || kind === 'tolerance' ? 'solve' : kind}|${st.match.source}` : null;
    follow.stageStart = now;
    follow.stageSize = st?.exp ? Math.max(1, n) ** st.exp : 1;
  }

  const st = plan[follow.index];
  let inner = 0;
  if (st) {
    const p = app.progress;
    const tol = (app.project.solve_params?.settings ?? DEFAULT_SOLVER_SETTINGS).gmres_tol;
    const g = /^GMRES/.test(stage) ? gmresFraction(p?.gmres, tol) : null;
    if (g !== null) inner = g;
    else if (p?.step && p.step[1] > 0 && !/^(level|case) /.test(job.stage ?? '')) inner = p.step[0] / p.step[1];
    else {
      const fallback = st.default ? st.default(Math.max(1, n)) : 2000;
      inner = eased(now - follow.stageStart, follow.stageKey ? expected(follow.stageKey, follow.stageSize, fallback) : fallback);
    }
  }
  const before = plan.slice(0, follow.index).reduce((a, s) => a + s.weight, 0);
  let f = (before + (st?.weight ?? 0) * Math.min(1, inner)) / total;
  if (level) f = (level[0] + f) / Math.max(1, level[1]);
  follow.best = Math.max(follow.best, Math.min(0.99, f));
  return follow.best;
}

/** The native server reports the command (open, upload) before the app names the job. */
const JOB_KIND_ALIASES: Record<string, string> = { open: 'load', upload: 'load', startup: 'load', import_project: 'load' };

// ------------------------------------------------------------------ quick commands

export interface Activity {
  id: number;
  key: string;
  label: string;
  work: number;
  started: number;
  /** expected duration (ms) */
  expect: number;
}

export const useActivity = create<{ items: Activity[] }>(() => ({ items: [] }));
let nextActivity = 1;

/**
 * Run `f`, showing it on the progress bar as `label` if it takes a while. `work` is its
 * size in the units its time grows with (grid points, sweep values, …): the bar expects
 * the time the same `key` took per unit before.
 */
export async function track<T>(key: string, label: string, work: number, f: () => Promise<T>): Promise<T> {
  const a: Activity = { id: nextActivity++, key, label, work: Math.max(work, 1e-9), started: performance.now(), expect: 0 };
  a.expect = expected(`cmd|${key}`, a.work, 1000);
  useActivity.setState((s) => ({ items: [...s.items, a] }));
  try {
    const r = await f();
    learn(`cmd|${key}`, performance.now() - a.started, a.work);
    return r;
  } finally {
    useActivity.setState((s) => ({ items: s.items.filter((x) => x.id !== a.id) }));
  }
}

/** A short busy step without its own progress ("Connecting", "Starting mesher"),
 *  timed like a quick command: call with the label while it shows, null when it ends. */
let busyNow: { label: string; started: number } | null = null;
export function busyFraction(label: string | null, now: number): number {
  if (busyNow && busyNow.label !== label) {
    learn(`busy|${busyNow.label}`, now - busyNow.started, 1);
    busyNow = null;
  }
  if (!label) return 0;
  busyNow ??= { label, started: now };
  return eased(now - busyNow.started, expected(`busy|${label}`, 1, 3000));
}

/** The quick command to show (the one expected to run longest), with its fraction. */
export function activityFraction(items: Activity[], now: number): { label: string; fraction: number } | null {
  if (!items.length) return null;
  const a = items.reduce((m, x) => (x.started + x.expect > m.started + m.expect ? x : m));
  return { label: a.label, fraction: eased(now - a.started, a.expect) };
}
