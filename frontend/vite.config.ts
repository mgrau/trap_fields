import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// WebAssembly threads (the in-browser engine) need SharedArrayBuffer, which browsers only
// enable on cross-origin isolated pages.
const isolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
};

// Dev server proxies the API to the Rust backend (`trapfields gui`, default port 8765);
// without one the app starts the in-browser engine.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    headers: isolation,
    proxy: { '/api': { target: `http://127.0.0.1:${process.env.TRAPFIELDS_PORT ?? 8765}` } },
  },
  preview: { headers: isolation },
  build: { chunkSizeWarningLimit: 1500 },
});
