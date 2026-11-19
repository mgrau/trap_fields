// Two ways to run the app: the native `trapfields` server (HTTP API) or the WebAssembly
// engine in a web worker (public/engine/). Both execute the same Rust application core;
// this module hides the difference from the rest of the GUI.
import type { AppStateDto } from './api';

export interface Backend {
  kind: 'server' | 'browser';
  /** human-readable description for the toolbar */
  label: string;
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

/** Commands that run in the background (the call returns at once, progress comes as events). */
const LONG = new Set(['startup', 'open', 'upload', 'mesh', 'solve', 'study', 'import_project']);

interface Live {
  kind: string;
  started: number;
  stage: string;
  gmres: [number, number[]][];
  study: unknown[];
  stopping: boolean;
}

type Reply = { id: number; json?: unknown; binary?: ArrayBuffer; error?: { status: number; message: string } };
type EventMsg = { type: string; kind?: string; stage?: string; iteration?: number; residuals?: number[]; [k: string]: unknown };

export class EngineBackend implements Backend {
  kind = 'browser' as const;
  label: string;
  private worker: Worker;
  private nextId = 1;
  private pending = new Map<number, { ok: (v: unknown) => void; fail: (e: Error) => void }>();
  private last: AppStateDto | null = null;
  private live: Live | null = null;
  /** the engine's stop flag: one byte in shared WebAssembly memory */
  private stopFlag: Uint8Array | null = null;
  private stopPtr = 0;

  private constructor(threads: number, onWaiting?: (why: string | null) => void) {
    this.label = `in browser (WebAssembly, ${threads} threads)`;
    this.worker = new Worker(`${import.meta.env.BASE_URL}engine/engine-worker.js`, { type: 'module' });
    this.worker.onmessage = ({ data }: MessageEvent<Reply & { event?: EventMsg; log?: string; waiting?: string | null }>) => {
      if (data.event) return this.onEvent(data.event);
      if ('waiting' in data) return onWaiting?.(data.waiting ?? null);
      if (data.log !== undefined) return console.debug('[engine]', data.log);
      const p = this.pending.get(data.id);
      if (!p) return;
      this.pending.delete(data.id);
      if (data.error) p.fail(Object.assign(new Error(data.error.message), { status: data.error.status }));
      else p.ok(data.json !== undefined ? data.json : data.binary);
    };
    this.worker.onerror = (e) => {
      const err = new Error(`engine crashed: ${e.message}`);
      for (const p of this.pending.values()) p.fail(err);
      this.pending.clear();
    };
  }

  static async start(onWaiting?: (why: string | null) => void): Promise<EngineBackend> {
    if (!self.crossOriginIsolated) throw new Error('the browser engine needs cross-origin isolation (COOP/COEP headers)');
    const threads = Math.max(1, (navigator.hardwareConcurrency || 4) - 2);
    const b = new EngineBackend(threads, onWaiting);
    const r = (await b.rpc({ init: { threads } })) as { stop?: { buffer: SharedArrayBuffer; ptr: number } };
    if (r.stop?.ptr) {
      b.stopFlag = new Uint8Array(r.stop.buffer);
      b.stopPtr = r.stop.ptr;
    }
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
      // the app clears its flag when a command starts: re-apply a stop requested before
      if (this.live.stopping) this.raiseStop();
    } else if (e.type === 'stage') this.live.stage = e.stage ?? '';
    else if (e.type === 'gmres') this.live.gmres.push([e.iteration ?? 0, e.residuals ?? []]);
    else if (e.type === 'study') this.live.study.push(e);
  }

  private view(): AppStateDto {
    const base = this.last ?? ({} as AppStateDto);
    if (!this.live) return base;
    return {
      ...base,
      job: {
        state: 'running',
        kind: this.live.kind,
        stage: this.live.stage,
        elapsed: (performance.now() - this.live.started) / 1000,
        stopping: this.live.stopping,
      },
      progress: { gmres: this.live.gmres, study: this.live.study },
    } as AppStateDto;
  }

  async cmd<T>(name: string, args?: unknown, data?: ArrayBuffer): Promise<T> {
    if (LONG.has(name)) {
      if (this.live) throw new Error(`busy: ${this.live.kind} is running`);
      const job: Live = { kind: name, started: performance.now(), stage: '', gmres: [], study: [], stopping: false };
      this.live = job;
      // only this job may clear `live`: another one may have started by the time it settles
      const done = () => {
        if (this.live === job) this.live = null;
      };
      this.rpc({ cmd: name, args, data }, data ? [data] : [])
        .then((st) => {
          this.last = st as AppStateDto;
          done();
        })
        .catch(async () => {
          // the job status (with the error) is part of the state
          done();
          try {
            this.last = (await this.rpc({ cmd: 'state' })) as AppStateDto;
          } catch {
            /* engine gone: the next state() call reports it */
          }
        });
      return this.view() as T;
    }
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
    if (!this.live) return;
    this.live.stopping = true;
    this.raiseStop();
  }

  async state(): Promise<AppStateDto> {
    if (this.live) return this.view();
    this.last = (await this.rpc({ cmd: 'state' })) as AppStateDto;
    return this.last;
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
