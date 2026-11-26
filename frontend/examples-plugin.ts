// Serves the example projects of ../examples (STEP file + trapfields.json each) as project
// bundles, the uncompressed ustar archives that "Open project…" reads: at
// <base>examples/<name>.tar from the dev server, and as files in the build. The app
// loads one with the import_project command, in the browser engine or the native server.
import fs from 'node:fs';
import path from 'node:path';
import type { Plugin } from 'vite';

const ROOT = path.resolve(__dirname, '../examples');
export const EXAMPLES = ['paul_trap', 'surface_trap', 'segmented_trap'];

function header(name: string, size: number): Buffer {
  const h = Buffer.alloc(512);
  const field = (off: number, len: number, v: number) => h.write(v.toString(8).padStart(len - 1, '0') + '\0', off, 'ascii');
  h.write(name, 0, 'utf8');
  field(100, 8, 0o644);
  field(108, 8, 0);
  field(116, 8, 0);
  field(124, 12, size);
  field(136, 12, 0);
  h.write('0', 156, 'ascii');
  h.write('ustar\0', 257, 'ascii');
  h.write('00', 263, 'ascii');
  h.write('        ', 148, 'ascii');
  let sum = 0;
  for (const b of h) sum += b;
  h.write(sum.toString(8).padStart(6, '0') + '\0 ', 148, 'ascii');
  return h;
}

/** The bundle of one example: its project file and the STEP file it names. */
export function bundle(name: string): Buffer {
  const dir = path.join(ROOT, name);
  const project = fs.readFileSync(path.join(dir, 'trapfields.json'));
  const step: string = JSON.parse(project.toString()).step_file;
  const parts: Buffer[] = [];
  for (const [file, data] of [['trapfields.json', project], [step, fs.readFileSync(path.join(dir, step))]] as const) {
    parts.push(header(file, data.length), data, Buffer.alloc((512 - (data.length % 512)) % 512));
  }
  parts.push(Buffer.alloc(1024));
  return Buffer.concat(parts);
}

export function examplesPlugin(): Plugin {
  let base = '/';
  return {
    name: 'trapfields-examples',
    configResolved(c) {
      base = c.base;
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const m = req.url?.split('?')[0].match(/\/examples\/([a-z_]+)\.tar$/);
        if (!m || !req.url!.startsWith(base) || !EXAMPLES.includes(m[1])) return next();
        res.setHeader('Content-Type', 'application/x-tar');
        res.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
        res.end(bundle(m[1]));
      });
    },
    generateBundle() {
      for (const name of EXAMPLES) this.emitFile({ type: 'asset', fileName: `examples/${name}.tar`, source: bundle(name) });
    },
  };
}
