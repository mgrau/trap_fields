// Derived state for results restored from the project (see api.results): pure helpers
// shared by the store and the Trap panels.
import type { SliceResult, SweepVar } from './api';

/** Axis label, display unit and scale (SI value / scale = displayed) of a swept parameter. */
export function sweepMeta(v: SweepVar): { label: string; unit: string; scale: number } {
  if (v.kind === 'rf_freq') return { label: 'RF frequency', unit: 'MHz', scale: 1e6 };
  if (v.kind === 'rf_amplitude') return { label: 'RF amplitude', unit: 'V', scale: 1 };
  return { label: `DC ${v.electrode}`, unit: 'V', scale: 1 };
}

/**
 * Color-scale reference (U at the ion, eV) and clip of a slice through the ion: the
 * value at the center cell (the slice is centered on the ion), and the 60th percentile
 * of U − U_ion unless `clip` is given.
 */
export function sliceScale(d: SliceResult, ref?: number, clip?: number | null): { ref: number; clip: number } {
  const vals = d.values.filter((v): v is number => v !== null);
  const mid = d.values[Math.floor(d.n / 2) * d.n + Math.floor(d.n / 2)];
  const r = ref ?? (mid !== null && mid !== undefined ? mid : Math.min(...vals));
  if (clip && clip > 0) return { ref: r, clip };
  const rel = vals.map((v) => v - r).sort((a, b) => a - b);
  return { ref: r, clip: rel[Math.floor(0.6 * (rel.length - 1))] || 1 };
}

/** Save a blob as a file. */
export function download(data: Blob, name: string) {
  const url = URL.createObjectURL(data);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
