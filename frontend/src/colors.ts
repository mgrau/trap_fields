import type { BodyState, Role } from './api';

// Tableau 10
const PALETTE = ['#4e79a7', '#f28e2b', '#e15759', '#76b7b2', '#59a14f', '#edc948', '#b07aa1', '#ff9da7', '#9c755f', '#bab0ac'];

export const ROLE_COLORS: Record<Role, string> = {
  unassigned: '#9aa0a6',
  rf: '#f76707',
  dc: '#4dabf7',
  ground: '#6c757d',
  floating: '#cc5de8',
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

export const MODE_COLORS = ['#ff922b', '#51cf66', '#339af0'];

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
