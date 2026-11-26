import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { examplesPlugin } from './examples-plugin';

// WebAssembly threads (the in-browser engine) need SharedArrayBuffer, which browsers only
// enable on cross-origin isolated pages.
const isolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

// Dev server proxies the API to the Rust backend (`trapfields gui`, default port 8765);
// without one the app starts the in-browser engine.
// TF_BASE: the path the app is served under (e.g. /trap_fields/ on GitHub Pages);
// VITE_STATIC=1 builds the browser-only app for a static host (see scripts/deploy-gh-pages.sh).
export default defineConfig({
  base: process.env.TF_BASE ?? '/',
  plugins: [react(), examplesPlugin()],
  server: {
    port: 5173,
    headers: isolation,
    proxy: { '/api': { target: `http://127.0.0.1:${process.env.TRAPFIELDS_PORT ?? 8765}` } },
  },
  preview: { headers: isolation },
  build: { chunkSizeWarningLimit: 1500 },
});
