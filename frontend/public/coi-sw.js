// Service worker for static hosts that cannot send custom headers (GitHub Pages): it
// re-serves this site's own responses with Cross-Origin-Opener-Policy and
// Cross-Origin-Embedder-Policy, which make the page cross-origin isolated —
// SharedArrayBuffer, and with it the threaded WebAssembly engine, needs that.
// Registered by src/isolation.ts in static builds only.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (new URL(req.url).origin !== self.location.origin) return;
  if (req.cache === 'only-if-cached' && req.mode !== 'same-origin') return;
  event.respondWith(
    fetch(req).then((res) => {
      if (res.status === 0) return res; // opaque: cannot be modified
      const headers = new Headers(res.headers);
      headers.set('Cross-Origin-Opener-Policy', 'same-origin');
      headers.set('Cross-Origin-Embedder-Policy', 'require-corp');
      headers.set('Cross-Origin-Resource-Policy', 'same-origin');
      return new Response(res.body, { status: res.status, statusText: res.statusText, headers });
    }),
  );
});
