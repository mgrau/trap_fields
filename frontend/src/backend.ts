// Two ways to run the app: the native `trapfields` server (HTTP API) or the WebAssembly
// engine in a web worker (public/engine/). Both execute the same Rust application core;
// this module hides the difference from the rest of the GUI.
import type { AppStateDto } from './api';

export interface Backend {
  kind: 'server' | 'browser';
  /** human-readable description for the toolbar */
  label: string;
  /** most memory the engine can use (bytes); null: the machine's (native server) */
  memoryLimit: number | null;
  /** a JSON command; long ones (open, mesh, solve, startup) return the state with the job running */
  cmd<T = unknown>(name: string, args?: unknown, data?: ArrayBuffer): Promise<T>;
  /** a binary result (tess, mesh_data, sigma) */
  bin(name: string): Promise<ArrayBuffer>;
  state(): Promise<AppStateDto>;
  /** stop the running long command at its next checkpoint */
  cancel(): Promise<void>;
}

// ---------------------------------------------------------------- native server

const ROUTES: Record<string, [string, string]> = {
  state: ['GET', '/state'],
  open: ['POST', '/open'],
  transform: ['POST', '/transform'],
  transform_undo: ['POST', '/transform/undo'],
  transform_reset: ['POST', '/transform/reset'],
  feature_faces: ['POST', '/features/faces'],
  remove_features: ['POST', '/features/remove'],
  features_undo: ['POST', '/features/undo'],
  features_reset: ['POST', '/features/reset'],
  labels: ['PUT', '/labels'],
  mesh: ['POST', '/mesh'],
  mesh_data: ['GET', '/mesh'],
  tess: ['GET', '/tess'],
  solve: ['POST', '/solve'],
  solution: ['GET', '/solve'],
  sigma: ['GET', '/solve/sigma'],
  eval: ['POST', '/solve/eval'],
  trap: ['POST', '/trap'],
  slice: ['POST', '/trap/slice'],
  study: ['POST', '/study'],
  moves: ['PUT', '/moves'],
  tolerance: ['POST', '/tolerance'],
  tolerance_eval: ['POST', '/tolerance/eval'],
  results: ['GET', '/results'],
  export_project: ['GET', '/export'],
  sweep: ['POST', '/sweep'],
  chain: ['POST', '/chain'],
};

async function http<T>(method: string, path: string, body?: BodyInit, json = true): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    // the server only accepts state-changing requests carrying this header (see tf-server api.rs)
    headers: body !== undefined && json ? { 'content-type': 'application/json', 'x-trapfields': '1' } : { 'x-trapfields': '1' },
    body,
  });
  if (!res.ok) {
    const text = await res.text();
    let msg = `${res.status} ${res.statusText}`;
    try {
      msg = JSON.parse(text).error ?? msg;
    } catch {
      /* not JSON */
    }
    throw new Error(msg);
  }
  const ct = res.headers.get('content-type') ?? '';
  return (ct.includes('json') ? res.json() : res.arrayBuffer()) as Promise<T>;
}

export class ServerBackend implements Backend {
  kind = 'server' as const;
  label = 'native server';
  memoryLimit = null;

  cmd<T>(name: string, args?: unknown, data?: ArrayBuffer): Promise<T> {
    if (name === 'upload') {
      const n = (args as { name: string }).name;
      return http<T>('POST', `/upload?name=${encodeURIComponent(n)}`, data, false);
    }
    if (name === 'import_project') return http<T>('POST', '/import', data, false);
    const r = ROUTES[name];
    if (!r) return Promise.reject(new Error(`unknown command ${name}`));
    return http<T>(r[0], r[1], r[0] === 'GET' || args === undefined ? undefined : JSON.stringify(args ?? null));
  }

  bin(name: string): Promise<ArrayBuffer> {
    return this.cmd<ArrayBuffer>(name);
  }

  state(): Promise<AppStateDto> {
    return this.cmd<AppStateDto>('state');
  }

  async cancel(): Promise<void> {
    await http('POST', '/cancel');
  }
}

// ---------------------------------------------------------------- browser engine

/**
 * Whether this browser runs 64-bit-memory (Memory64) WebAssembly with shared memory, as
 * the wasm64 engine needs: a module whose only content is a shared 64-bit memory
 * (memory section: flags 7 = has maximum | shared | 64-bit, minimum 0, maximum 1).
 */
export function memory64Supported(): boolean {
  try {
    return WebAssembly.validate(new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 5, 4, 1, 7, 0, 1]));
  } catch {
    return false;
  }
}

/** Memory limits of the two engine builds. */
const MEMORY_32 = 4 * 2 ** 30;
const MEMORY_64 = 16 * 2 ** 30;

/** Commands that run in the background (the call returns at once, progress comes as events). */
const LONG = new Set(['startup', 'open', 'upload', 'mesh', 'solve', 'study', 'tolerance', 'import_project', 'remove_features', 'features_undo', 'features_reset']);

interface Live {
  /** the command, as sent */
  cmd: string;
  args: unknown;
  kind: string;
  started: number;
  stage: string;
  gmres: [number, number[]][];
  study: unknown[];
  step: [number, number] | null;
  stopping: boolean;
  /** the command finished and the worker is saving the project to IndexedDB */
  syncing: boolean;
  /** stopped by terminating the engine: the command's reply no longer matters */
  restarting: boolean;
}

/**
 * How long a stop may take to land before the engine is restarted. A single gmsh call
 * (the OpenCASCADE import of a big STEP file, one meshing pass) cannot be interrupted and
 * can run for minutes; checkpoints elsewhere are well under a second apart.
 */
const STOP_GRACE_MS = 3000;

type Reply = { id: number; json?: unknown; binary?: ArrayBuffer; error?: { status: number; message: string } };
type EventMsg = { type: string; kind?: string; stage?: string; iteration?: number; residuals?: number[]; done?: number; total?: number; [k: string]: unknown };
type WorkerMsg = Reply & { event?: EventMsg; log?: string; waiting?: string | null; syncing?: boolean };

/** The file a load command opens, if it names one. */
function loadTarget(cmd: string, args: unknown): string | null {
  const a = (args ?? {}) as { file?: string; name?: string };
  if (cmd === 'open') return a.file ?? null;
  if (cmd === 'upload') return a.name ?? null;
  return null;
}

/** Job kind the app reports for a long command. */
const JOB_KIND: Record<string, string> = { startup: 'load', open: 'load', upload: 'load', import_project: 'load' };

export class EngineBackend implements Backend {
  kind = 'browser' as const;
  label = '';
  memoryLimit: number = MEMORY_32;
  private worker!: Worker;
  private threads: number;
  /** run the 64-bit-memory engine (falls back to the 32-bit one if it does not start) */
  private memory64: boolean;
  private onWaiting?: (why: string | null) => void;
  private nextId = 1;
  private pending = new Map<number, { ok: (v: unknown) => void; fail: (e: Error) => void }>();
  private last: AppStateDto | null = null;
  private live: Live | null = null;
  /** the engine's stop flag: one byte in shared WebAssembly memory */
  private stopFlag: Uint8Array | null = null;
  private stopPtr = 0;
  private stopTimer: number | undefined;
  /** what happened to a job that had to be stopped by restarting the engine; reported
   * with the state until the next long command (the GUI shows each one once) */
  private notice: string | null = null;

  private constructor(threads: number, memory64: boolean, onWaiting?: (why: string | null) => void) {
    this.threads = threads;
    this.memory64 = memory64;
    this.onWaiting = onWaiting;
    this.setLabel();
  }

  private setLabel() {
    this.memoryLimit = this.memory64 ? MEMORY_64 : MEMORY_32;
    this.label = `in browser (WebAssembly ${this.memory64 ? '64' : '32'}-bit, ${this.threads} threads, up to ${this.memoryLimit / 2 ** 30} GB)`;
  }

  /**
   * Create the worker and start the engine in it (it opens the project in IndexedDB).
   * `restart`: the project lock may still be held for a moment by the terminated
   * worker; that is not "another tab", so it is not reported.
   */
  private async boot(restart = false): Promise<void> {
    const w = new Worker(`${import.meta.env.BASE_URL}engine/engine-worker.js`, { type: 'module' });
    this.worker = w;
    w.onmessage = ({ data }: MessageEvent<WorkerMsg>) => {
      if (w !== this.worker) return; // a terminated engine's last words
      if (data.event) return this.onEvent(data.event);
      if ('waiting' in data) return restart ? undefined : this.onWaiting?.(data.waiting ?? null);
      if (data.syncing) {
        if (this.live) this.live.syncing = true;
        return;
      }
      if (data.log !== undefined) return console.debug('[engine]', data.log);
      const p = this.pending.get(data.id);
      if (!p) return;
      this.pending.delete(data.id);
      if (data.error) p.fail(Object.assign(new Error(data.error.message), { status: data.error.status }));
      else p.ok(data.json !== undefined ? data.json : data.binary);
    };
    w.onerror = (e) => {
      if (w === this.worker) this.failPending(new Error(`engine crashed: ${e.message}`));
    };
    this.stopFlag = null;
    let r: { stop?: { buffer: SharedArrayBuffer; ptr: number } };
    try {
      r = (await this.rpc({ init: { threads: this.threads, memory64: this.memory64 } })) as typeof r;
    } catch (e) {
      if (!this.memory64) throw e;
      // no wasm64 engine in this build, or it failed to start: use the 32-bit one
      console.warn('[engine] the 64-bit engine did not start, using the 32-bit one:', e);
      w.terminate();
      this.memory64 = false;
      this.setLabel();
      // the terminated worker may hold the project lock a moment longer: not "another tab"
      return this.boot(true);
    }
    if (r.stop?.ptr) {
      this.stopFlag = new Uint8Array(r.stop.buffer);
      this.stopPtr = r.stop.ptr;
    }
  }

  private failPending(err: Error) {
    for (const p of this.pending.values()) p.fail(err);
    this.pending.clear();
  }

  static async start(onWaiting?: (why: string | null) => void): Promise<EngineBackend> {
    if (!self.crossOriginIsolated) throw new Error('the browser engine needs cross-origin isolation (COOP/COEP headers)');
    const threads = Math.max(1, (navigator.hardwareConcurrency || 4) - 2);
    // ?wasm=32 / ?wasm=64 force an engine build
    const force = new URLSearchParams(location.search).get('wasm');
    const b = new EngineBackend(threads, force === '64' || (force !== '32' && memory64Supported()), onWaiting);
    await b.boot();
    // a full state before any long command, so the GUI always has one to render
    b.last = (await b.rpc({ cmd: 'state' })) as AppStateDto;
    return b;
  }

  private rpc(msg: Record<string, unknown>, transfer: Transferable[] = []): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((ok, fail) => {
      this.pending.set(id, { ok, fail });
      this.worker.postMessage({ id, ...msg }, transfer);
    });
  }

  private onEvent(e: EventMsg) {
    if (!this.live) return;
    if (e.type === 'start') {
      this.live.kind = e.kind ?? this.live.kind;
      this.live.stage = '';
      this.live.gmres = [];
      this.live.step = null;
      // the app clears its flag when a command starts: re-apply a stop requested before
      if (this.live.stopping) this.raiseStop();
    } else if (e.type === 'stage') {
      this.live.stage = e.stage ?? '';
      this.live.step = null;
    } else if (e.type === 'gmres') this.live.gmres.push([e.iteration ?? 0, e.residuals ?? []]);
    else if (e.type === 'study') {
      this.live.study.push(e);
      this.live.gmres = [];
    } else if (e.type === 'progress') this.live.step = [e.done ?? 0, e.total ?? 0];
  }

  private view(): AppStateDto {
    const base = this.last ?? ({} as AppStateDto);
    if (!this.live) return base;
    return {
      ...base,
      job: {
        state: 'running',
        kind: this.live.kind,
        stage: this.live.syncing ? 'saving the project' : this.live.stage,
        elapsed: (performance.now() - this.live.started) / 1000,
        stopping: this.live.stopping,
      },
      progress: { gmres: this.live.gmres, study: this.live.study, step: this.live.step },
    } as AppStateDto;
  }

  private withNotice(st: AppStateDto): AppStateDto {
    return this.notice ? { ...st, notice: this.notice } : st;
  }

  /** Run a long command in the background; the returned state shows it running. */
  private startLong(name: string, args?: unknown, data?: ArrayBuffer): AppStateDto {
    if (this.live) throw new Error(`busy: ${this.live.kind} is running`);
    const job: Live = { cmd: name, args, kind: JOB_KIND[name] ?? name, started: performance.now(), stage: '', gmres: [], study: [], step: null, stopping: false, syncing: false, restarting: false };
    this.live = job;
    this.notice = null;
    window.clearTimeout(this.stopTimer);
    this.rpc({ cmd: name, args, data }, data ? [data] : [])
      .then((st) => {
        // only this job may clear `live`: another one (or a restarted engine's) may own it now
        if (this.live !== job || job.restarting) return;
        this.last = st as AppStateDto;
        this.live = null;
      })
      .catch(async () => {
        if (this.live !== job || job.restarting) return;
        this.live = null;
        // the job status (with the error) is part of the state
        try {
          this.last = (await this.rpc({ cmd: 'state' })) as AppStateDto;
        } catch {
          /* engine gone: the next state() call reports it */
        }
      });
    return this.view();
  }

  async cmd<T>(name: string, args?: unknown, data?: ArrayBuffer): Promise<T> {
    if (LONG.has(name)) return this.startLong(name, args, data) as T;
    const r = await this.rpc({ cmd: name, args, data }, data ? [data] : []);
    // commands that change the project reply with the new state
    if (r && typeof r === 'object' && 'project' in (r as object) && 'job' in (r as object)) this.last = r as AppStateDto;
    return r as T;
  }

  bin(name: string): Promise<ArrayBuffer> {
    return this.rpc({ cmd: name }) as Promise<ArrayBuffer>;
  }

  private raiseStop() {
    if (this.stopFlag) Atomics.store(this.stopFlag, this.stopPtr, 1);
  }

  async cancel(): Promise<void> {
    const job = this.live;
    if (!job) return;
    job.stopping = true;
    this.raiseStop();
    // a stop that does not land soon is stuck in a call that cannot be interrupted
    window.clearTimeout(this.stopTimer);
    const check = () => {
      if (this.live !== job) return;
      // never kill the worker while it writes the project to IndexedDB
      if (job.syncing) this.stopTimer = window.setTimeout(check, 500);
      else void this.restart(job);
    };
    this.stopTimer = window.setTimeout(check, STOP_GRACE_MS);
  }

  /**
   * Stop `job` by terminating the engine and starting a fresh one. Nothing the job did
   * reaches IndexedDB (the worker saves the project only after a command finishes), so
   * the project is as it was before the job: an uploaded file is not kept, and the
   * project's STEP file is still the previous one.
   */
  private async restart(job: Live): Promise<void> {
    job.stage = 'restarting the engine';
    job.restarting = true;
    this.worker.terminate();
    this.stopFlag = null;
    this.failPending(new Error('the engine was restarted'));
    try {
      await this.boot(true);
      const st = (await this.rpc({ cmd: 'state' })) as AppStateDto;
      // Re-open the project's model only if it is not the one that was stuck loading:
      // a huge file that was loaded once before would otherwise hang every restart too.
      const target = loadTarget(job.cmd, job.args);
      const file = st.project.step_file;
      const reopen = job.cmd !== 'startup' && !!file && file !== target && st.files.includes(file);
      this.last = { ...st, job: { state: 'stopped', kind: job.kind } };
      if (this.live === job) this.live = null;
      let notice = 'Stopped. The operation could not be interrupted, so the engine was restarted.';
      if (reopen) {
        notice += ` Re-opening ${file}.`;
        this.startLong('startup');
      } else if (file) {
        notice += ` ${file} was not re-opened: choose it under Project to load it.`;
      }
      this.notice = notice;
    } catch (e) {
      if (this.live === job) this.live = null;
      const error = `The engine did not restart after the stop: ${(e as Error).message}. Reload the page.`;
      if (this.last) this.last = { ...this.last, job: { state: 'failed', kind: job.kind, error } };
      this.notice = error;
    }
  }

  async state(): Promise<AppStateDto> {
    if (this.live) return this.withNotice(this.view());
    this.last = (await this.rpc({ cmd: 'state' })) as AppStateDto;
    return this.withNotice(this.last);
  }
}

// ---------------------------------------------------------------- selection

let current: Backend | null = null;
let connecting: Promise<Backend> | null = null;
let failure: Error | null = null;

/** The connected backend; throws, with the reason, if there is none (yet). */
export function backend(): Backend {
  if (current) return current;
  throw new Error(failure ? `The computation engine did not start: ${failure.message}` : 'The computation engine is still starting.');
}

/** The backend, waiting for it while it starts. */
export async function whenConnected(): Promise<Backend> {
  if (!current && connecting) {
    try {
      await connecting;
    } catch {
      /* reported by backend() */
    }
  }
  return backend();
}

/** Backend calls that wait for the connection (requests made while it starts, e.g. a file
 * dropped right after the page opens, are held instead of failing). */
export const connected = {
  cmd<T = unknown>(name: string, args?: unknown, data?: ArrayBuffer): Promise<T> {
    return whenConnected().then((x) => x.cmd<T>(name, args, data));
  },
  bin(name: string): Promise<ArrayBuffer> {
    return whenConnected().then((x) => x.bin(name));
  },
  state(): Promise<AppStateDto> {
    return whenConnected().then((x) => x.state());
  },
  cancel(): Promise<void> {
    return whenConnected().then((x) => x.cancel());
  },
};

/**
 * Use the native server when one answers at /api (the `trapfields gui` command), else
 * start the in-browser engine. `?engine=browser` / `?engine=server` forces a choice.
 * `onStatus` hears why starting takes long (another tab holds the browser's project).
 */
export function connect(onStatus?: (message: string | null) => void): Promise<Backend> {
  connecting ??= (async () => {
    const force = new URLSearchParams(location.search).get('engine');
    // a static build (GitHub Pages) has no server to ask
    if (force !== 'browser' && import.meta.env.VITE_STATIC !== '1') {
      try {
        const res = await fetch('/api/state', { cache: 'no-store', signal: AbortSignal.timeout(2000) });
        if (res.ok && (res.headers.get('content-type') ?? '').includes('json')) return (current = new ServerBackend());
      } catch {
        /* no server */
      }
      if (force === 'server') throw new Error('no trapfields server answers at /api');
    }
    const engine = await EngineBackend.start((why) =>
      onStatus?.(why === 'another-tab' ? 'Waiting for trapfields in another tab of this browser to close (one tab at a time)' : null),
    );
    await engine.cmd('startup');
    return (current = engine);
  })().catch((e: Error) => {
    failure = e;
    throw e;
  });
  return connecting;
}
