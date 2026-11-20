import type { BodyState, Role } from './api';

/**
 * Categorical palette (light-mode steps), in fixed order, for the white panel surface.
 * The dark-mode steps for figures exported on a dark background are in
 * plot/exportFigure.ts. Electrodes use it in the 3D view and in plots, so an electrode keeps one
 * color everywhere. More than 8 series fold or facet; never generate a 9th hue.
 */
export const SERIES = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#6250d6', '#e34948'];
const PALETTE = SERIES;

export const ROLE_COLORS: Record<Role, string> = {
  unassigned: '#94a3b8',
  rf: '#ea580c',
  dc: '#2563eb',
  ground: '#64748b',
  floating: '#a855f7',
};

export const ROLE_NAMES: Record<Role, string> = {
  unassigned: 'Unassigned',
  rf: 'RF',
  dc: 'DC',
  ground: 'Ground',
  floating: 'Floating',
};

/** One color per body: bodies of the same electrode share a color. */
export function bodyColors(bodies: BodyState[], by: 'electrode' | 'role'): string[] {
  if (by === 'role') return bodies.map((b) => ROLE_COLORS[b.label?.role ?? 'unassigned']);
  const order: string[] = [];
  for (const b of bodies) {
    const e = b.label?.electrode ?? b.name;
    if (!order.includes(e)) order.push(e);
  }
  return bodies.map((b) => PALETTE[order.indexOf(b.label?.electrode ?? b.name) % PALETTE.length]);
}

/** Color per electrode name, in first-appearance order over the bodies (as in the viewer). */
export function electrodeColors(bodies: BodyState[]): Record<string, string> {
  const out: Record<string, string> = {};
  let k = 0;
  for (const b of bodies) {
    const e = b.label?.electrode ?? b.name;
    if (!(e in out)) out[e] = PALETTE[k++ % PALETTE.length];
  }
  return out;
}

/** Viridis colormap, t in [0, 1] -> [r, g, b] in [0, 1]. */
const VIRIDIS = [
  [0.267, 0.005, 0.329],
  [0.283, 0.141, 0.458],
  [0.254, 0.265, 0.53],
  [0.207, 0.372, 0.553],
  [0.164, 0.471, 0.558],
  [0.128, 0.567, 0.551],
  [0.135, 0.659, 0.518],
  [0.267, 0.749, 0.441],
  [0.478, 0.821, 0.318],
  [0.741, 0.873, 0.15],
  [0.993, 0.906, 0.144],
];

export function viridis(t: number): [number, number, number] {
  if (!Number.isFinite(t)) t = 0; // NaN would index past the table
  const x = Math.min(1, Math.max(0, t)) * (VIRIDIS.length - 1);
  const i = Math.min(VIRIDIS.length - 2, Math.floor(x));
  const f = x - i;
  const a = VIRIDIS[i];
  const b = VIRIDIS[i + 1];
  return [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), a[2] + f * (b[2] - a[2])];
}

export const viridisCss = (t: number) => {
  const [r, g, b] = viridis(t);
  return `rgb(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)})`;
};

/** Diverging blue–white–red, t in [-1, 1] -> [r, g, b] in [0, 1]. */
const DIVERGING = [
  [0.019, 0.188, 0.38],
  [0.129, 0.4, 0.675],
  [0.263, 0.576, 0.765],
  [0.573, 0.773, 0.871],
  [0.82, 0.898, 0.941],
  [0.969, 0.969, 0.969],
  [0.992, 0.859, 0.78],
  [0.957, 0.647, 0.51],
  [0.839, 0.376, 0.302],
  [0.698, 0.094, 0.169],
  [0.404, 0.0, 0.122],
];

export function diverging(t: number): [number, number, number] {
  if (!Number.isFinite(t)) t = 0;
  const x = ((Math.min(1, Math.max(-1, t)) + 1) / 2) * (DIVERGING.length - 1);
  const i = Math.min(DIVERGING.length - 2, Math.floor(x));
  const f = x - i;
  const a = DIVERGING[i];
  const b = DIVERGING[i + 1];
  return [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), a[2] + f * (b[2] - a[2])];
}

export const divergingCss = (t: number) => {
  const [r, g, b] = diverging(t);
  return `rgb(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)})`;
};

/** Principal axes / secular modes: the first three palette slots. */
export const MODE_COLORS = [SERIES[0], SERIES[1], SERIES[2]];

/** Side (px) the slice image is resampled to: about what a large plot shows at 2× DPR. */
const SLICE_IMAGE_SIDE = 1000;

/**
 * RGBA image (size × size, row 0 = lowest v) of an energy slice on an n×n node grid:
 * viridis over [ref, ref + clip] eV with contour lines every clip/10, translucent above
 * the clip, transparent inside electrodes. Each node owns a 1/n-wide cell, as in a
 * pixel-per-node image, but the values are interpolated bilinearly at a finer
 * resolution and the contours are drawn at a constant ~1.3 px width from the distance
 * to the level (|f − k| / |∇f|), so neither the cells nor stair-stepped contours show.
 */
export function sliceImage(values: (number | null)[], n: number, ref: number, clip: number): { rgba: Uint8Array; size: number } {
  // the 2D map and the 3D plane draw the same slice: build it once
  const c = lastSlice;
  if (c && c.values === values && c.ref === ref && c.clip === clip) return c.image;
  const image = buildSliceImage(values, n, ref, clip);
  lastSlice = { values, ref, clip, image };
  return image;
}

let lastSlice: { values: (number | null)[]; ref: number; clip: number; image: { rgba: Uint8Array; size: number } } | null = null;

function buildSliceImage(values: (number | null)[], n: number, ref: number, clip: number): { rgba: Uint8Array; size: number } {
  const k =Math.max(1, Math.ceil(SLICE_IMAGE_SIDE / n));
  const m = n * k;
  // level coordinate f = 10 (U − ref)/clip at each fine pixel; NaN inside electrodes
  const f = new Float32Array(m * m);
  const node = (q: number) => {
    const x = Math.min(n - 1, Math.max(0, (q + 0.5) / k - 0.5));
    const i = Math.min(n - 2, Math.floor(x));
    return [i, x - i] as const;
  };
  const cols = Array.from({ length: m }, (_, q) => node(q));
  for (let q = 0; q < m; q++) {
    const [j, fy] = cols[q];
    for (let p = 0; p < m; p++) {
      const [i, fx] = cols[p];
      const c = j * n + i;
      const v = [values[c], values[c + 1], values[c + n], values[c + n + 1]];
      const w = [(1 - fx) * (1 - fy), fx * (1 - fy), (1 - fx) * fy, fx * fy];
      let sw = 0;
      let sv = 0;
      for (let t = 0; t < 4; t++) {
        const x = v[t];
        if (x !== null && x !== undefined) {
          sw += w[t];
          sv += w[t] * x;
        }
      }
      // electrode edges run between nodes: a pixel is outside while most of its weight is
      f[q * m + p] = sw >= 0.5 ? (10 * (sv / sw - ref)) / clip : NaN;
    }
  }
  const out = new Uint8Array(4 * m * m);
  const d = (a: number, b: number, h: number) => (Number.isFinite(a) && Number.isFinite(b) ? (b - a) / h : NaN);
  for (let q = 0; q < m; q++) {
    for (let p = 0; p < m; p++) {
      const at = q * m + p;
      const y = f[at];
      if (!Number.isFinite(y)) continue;
      const [r, g, b] = viridis(y / 10);
      // NaN-safe central differences, one-sided at edges and next to electrodes
      const slope = (lo: number, hi: number) => {
        const c = d(lo, hi, 2);
        if (Number.isFinite(c)) return c;
        const l = d(lo, y, 1);
        if (Number.isFinite(l)) return l;
        const h = d(y, hi, 1);
        return Number.isFinite(h) ? h : 0;
      };
      const grad = Math.hypot(
        slope(p > 0 ? f[at - 1] : NaN, p + 1 < m ? f[at + 1] : NaN),
        slope(q > 0 ? f[at - m] : NaN, q + 1 < m ? f[at + m] : NaN),
      );
      let line = 0;
      const lv = Math.round(y);
      if (lv >= 1 && lv <= 10 && grad > 0) {
        const dist = Math.abs(y - lv) / grad;
        // fade lines where levels crowd closer than ~2 px, rather than a solid dark band
        line = Math.max(0, Math.min(1, 1.3 - dist)) * Math.min(1, 0.5 / grad);
      }
      const s = 1 - 0.55 * line;
      const o = 4 * at;
      out[o] = r * 255 * s;
      out[o + 1] = g * 255 * s;
      out[o + 2] = b * 255 * s;
      out[o + 3] = y > 10 ? 70 + 165 * line : 235;
    }
  }
  return { rgba: out, size: m };
}

/** Ink for plot markings that are not series: muted references and the RF null marker. */
export const INK_MUTED = '#64748b';
export const INK_STRONG = '#0f172a';
