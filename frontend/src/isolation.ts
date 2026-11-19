// Cross-origin isolation on static hosts. The native server and `vite dev/preview` send
// the COOP/COEP headers themselves; a static build (VITE_STATIC=1, e.g. for GitHub Pages)
// gets them from a service worker (public/coi-sw.js), which only applies after a reload.

const KEY = 'tf.isolation-reloads';

function reloads(): number {
  try {
    return Number(sessionStorage.getItem(KEY) ?? 0);
  } catch {
    return 0;
  }
}

function setReloads(n: number | null) {
  try {
    if (n === null) sessionStorage.removeItem(KEY);
    else sessionStorage.setItem(KEY, String(n));
  } catch {
    /* storage unavailable: at worst one extra reload is skipped */
  }
}

/** Resolves to true when the page should render now, false when it is reloading. */
export async function ensureIsolation(): Promise<boolean> {
  if (self.crossOriginIsolated) {
    setReloads(null);
    return true;
  }
  if (import.meta.env.VITE_STATIC !== '1' || !('serviceWorker' in navigator)) return true;
  // give up after two attempts (e.g. private windows without service workers): the app
  // then explains that the engine needs cross-origin isolation
  const n = reloads();
  if (n >= 2) return true;
  try {
    await navigator.serviceWorker.register(`${import.meta.env.BASE_URL}coi-sw.js`, { scope: import.meta.env.BASE_URL });
    await navigator.serviceWorker.ready;
  } catch (e) {
    console.warn('cross-origin isolation service worker unavailable:', e);
    return true;
  }
  setReloads(n + 1);
  location.reload();
  return false;
}
