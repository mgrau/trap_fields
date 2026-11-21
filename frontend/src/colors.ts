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

/**
 * RGBA texture for an energy slice: viridis over [ref, ref + clip] eV with contour lines
 * every clip/10, translucent above the clip, transparent where values are null.
 */
export function sliceRgba(values: (number | null)[], n: number, ref: number, clip: number): Uint8Array {
  const out = new Uint8Array(4 * n * n);
  const level = (v: number | null) => (v === null ? -1 : Math.floor(((v - ref) / clip) * 10));
  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) {
      const k = j * n + i;
      const v = values[k];
      if (v === null) continue;
      const t = (v - ref) / clip;
      const [r, g, b] = viridis(t);
      const lv = level(v);
      const contour = lv < 10 && ((i + 1 < n && level(values[k + 1]) !== lv && values[k + 1] !== null) || (j + 1 < n && level(values[k + n]) !== lv && values[k + n] !== null));
      const f = contour ? 0.45 : 1;
      out.set([r * 255 * f, g * 255 * f, b * 255 * f, t > 1 ? 70 : 235], 4 * k);
    }
  }
  return out;
}

/** Ink for plot markings that are not series: muted references and the RF null marker. */
export const INK_MUTED = '#64748b';
export const INK_STRONG = '#0f172a';
