// Web worker hosting the trapfields WebAssembly engine (tf_engine.js / .wasm, or the
// 64-bit-memory tf_engine64.js / .wasm, built by scripts/build-wasm-engine.sh). The page
// sends {id, cmd, args, data} or {id, init: {threads, memory64}};
// replies are {id, json} | {id, binary} | {id, error}; events from long commands are
// {event}; {syncing: true} says a command finished and its changes are being saved. The
// page stops a command stuck in gmsh by terminating this worker: nothing that command
// changed reaches IndexedDB, which is only written after a command. The project
// directory /project lives in IndexedDB (Emscripten IDBFS) and is synced after every
// command that can change it.
let M = null;
// wasm64: the exports take and return pointers and sizes as 64-bit integers (BigInt)
let wide = false;
const P = (x) => (wide ? BigInt(x) : x);
const enc = new TextEncoder();
const dec = new TextDecoder();
// commands that never write to /project (no IndexedDB sync needed afterwards)
const READ_ONLY = new Set(['state', 'tess', 'mesh_data', 'solution', 'sigma', 'eval', 'results', 'export_project', 'export_fields']);

const heap = () => new Uint8Array(M.wasmMemory.buffer);

function put(bytes) {
  const p = Number(M._tf_alloc(P(bytes.length)));
  heap().set(bytes, p);
  return p;
}

function output() {
  const p = Number(M._tf_out_ptr());
  const n = Number(M._tf_out_len());
  return heap().slice(p, p + n); // a copy, not a view of shared memory
}

function sync(populate) {
  return new Promise((ok, fail) => M.FS.syncfs(populate, (e) => (e ? fail(e) : ok())));
}

/** Hold the project lock while this worker lives: two tabs would overwrite each other's
 * project in IndexedDB (every sync writes this tab's whole copy). If another tab holds
 * it, tell the page and wait until that tab closes. */
async function lockProject() {
  if (!self.navigator || !navigator.locks) return;
  const take = (options) =>
    new Promise((resolve) => {
      navigator.locks.request('trapfields-project', options, (lock) => {
        resolve(!!lock);
        return lock ? new Promise(() => {}) : undefined; // held for the worker's lifetime
      });
    });
  if (await take({ ifAvailable: true })) return;
  postMessage({ waiting: 'another-tab' });
  await take({});
  postMessage({ waiting: null });
}

async function init(threads, memory64) {
  await lockProject();
  const { default: createEngine } = await import(memory64 ? './tf_engine64.js' : './tf_engine.js');
  M = await createEngine({
    onEvent: (event) => postMessage({ event }),
    print: (s) => postMessage({ log: s }),
    printErr: (s) => postMessage({ log: s }),
  });
  wide = typeof M._tf_out_len() === 'bigint';
  M.FS.mkdir('/project');
  M.FS.mount(M.IDBFS, {}, '/project');
  await sync(true);
  const dir = enc.encode('/project');
  const p = put(dir);
  const rc = M._tf_init(P(p), P(dir.length), P(threads));
  M._tf_dealloc(P(p), P(dir.length));
  if (rc !== 0) throw new Error(dec.decode(output()));
}

function command(cmd, args, data) {
  const c = enc.encode(cmd);
  const a = enc.encode(JSON.stringify(args ?? null));
  const b = data ? new Uint8Array(data) : new Uint8Array(0);
  const [pc, pa, pb] = [put(c), put(a), put(b)];
  try {
    return [M._tf_command(P(pc), P(c.length), P(pa), P(a.length), P(pb), P(b.length)), output()];
  } finally {
    M._tf_dealloc(P(pc), P(c.length));
    M._tf_dealloc(P(pa), P(a.length));
    M._tf_dealloc(P(pb), P(b.length));
  }
}

// one command at a time, in arrival order
let queue = Promise.resolve();
self.onmessage = ({ data: msg }) => {
  queue = queue.then(async () => {
    const { id } = msg;
    try {
      if (msg.init) {
        await init(msg.init.threads, !!msg.init.memory64);
        // the stop flag lives in shared WebAssembly memory: the page sets it directly
        postMessage({ id, json: { threads: msg.init.threads, memory64: !!msg.init.memory64, stop: { buffer: M.wasmMemory.buffer, ptr: Number(M._tf_stop_ptr()) } } });
        return;
      }
      const [kind, out] = command(msg.cmd, msg.args, msg.data);
      if (!READ_ONLY.has(msg.cmd)) {
        // the page may terminate a stuck worker to stop it, but not while it saves
        postMessage({ syncing: true });
        await sync(false);
      }
      if (kind === 2) postMessage({ id, error: JSON.parse(dec.decode(out)) });
      else if (kind === 0) postMessage({ id, json: JSON.parse(dec.decode(out)) });
      else postMessage({ id, binary: out.buffer }, [out.buffer]);
    } catch (e) {
      postMessage({ id, error: { status: 500, message: String(e && e.message ? e.message : e) } });
    }
  });
};
