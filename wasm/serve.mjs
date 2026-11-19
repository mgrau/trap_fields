// Minimal static server for the WASM test pages (correct MIME types, localhost only).
// Sends the cross-origin isolation headers that SharedArrayBuffer (WebAssembly threads) needs.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
const root = process.argv[2];
const port = Number(process.argv[3] ?? 8790);
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.step': 'application/octet-stream' };
createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(/^(\.\.[/\\])+/, '');
  try {
    const body = await readFile(join(root, path.endsWith('/') ? path + 'index.html' : path));
    res.writeHead(200, {
      'content-type': types[extname(path)] ?? (path.endsWith('/') ? 'text/html' : 'application/octet-stream'),
      'cross-origin-opener-policy': 'same-origin',
      'cross-origin-embedder-policy': 'require-corp',
      'cross-origin-resource-policy': 'same-origin',
    });
    res.end(body);
  } catch {
    res.writeHead(404).end('not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`serving ${root} on http://127.0.0.1:${port}`));
