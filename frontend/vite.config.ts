import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Dev server proxies the API to the Rust backend (`trapfields gui`, default port 8765).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { '/api': { target: `http://127.0.0.1:${process.env.TRAPFIELDS_PORT ?? 8765}` } },
  },
  build: { chunkSizeWarningLimit: 1500 },
});
