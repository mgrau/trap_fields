// Export of plots as standalone figures: SVG (vector), PNG (2× raster), clipboard image,
// and the data as CSV / tab-separated text. A figure is the plot's SVG plus its title,
// legend and color bar, which on screen are HTML around it.
import { download } from '../results';

export type FigureTheme = 'dark' | 'light';

export interface LegendItem {
  name: string;
  color: string;
  dash?: boolean;
}

export interface ColorBar {
  /** CSS colors from low to high */
  stops: string[];
  lo: string;
  hi: string;
  label: string;
}

export interface FigureMeta {
  title?: string;
  legend?: LegendItem[];
  colorBar?: ColorBar;
}

const INK = {
  dark: { bg: '#1c2027', text: '#dfe3ea', muted: '#8b93a1', grid: '#2f3540', axis: '#444c59', ref: '#6b7280' },
  light: { bg: '#ffffff', text: '#1f2328', muted: '#57606a', grid: '#e4e7eb', axis: '#9aa1ab', ref: '#7d8590' },
};

/** On-screen (light-mode) categorical steps (colors.ts SERIES) → their dark-mode steps, plus marker inks. */
const DARK_STEPS: Record<string, string> = {
  '#2a78d6': '#3987e5',
  '#eb6834': '#d95926',
  '#1baf7a': '#199e70',
  '#eda100': '#c98500',
  '#e87ba4': '#d55181',
  '#008300': '#0ca30c',
  '#6250d6': '#9085e9',
  '#e34948': '#e66767',
  '#64748b': '#8b93a1',
  '#0f172a': '#ffffff',
  '#ffffff': '#1c2027',
};

export function themeColor(c: string, theme: FigureTheme): string {
  return theme === 'dark' ? (DARK_STEPS[c.toLowerCase()] ?? c) : c;
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const FONT = "system-ui, -apple-system, 'Segoe UI', Helvetica, Arial, sans-serif";

/**
 * A standalone SVG document of `svg` (a rendered plot) with title, legend and color bar.
 * Elements marked data-export="skip" (hover layers) are left out.
 */
export function figureSvg(svg: SVGSVGElement, meta: FigureMeta, theme: FigureTheme): string {
  const ink = INK[theme];
  const w = svg.width.baseVal.value || svg.getBoundingClientRect().width;
  const h = svg.height.baseVal.value || svg.getBoundingClientRect().height;
  const clone = svg.cloneNode(true) as SVGSVGElement;
  clone.querySelectorAll('[data-export="skip"]').forEach((n) => n.remove());
  let inner = clone.innerHTML;
  if (theme === 'dark') inner = inner.replace(/#[0-9a-fA-F]{6}\b/g, (c) => DARK_STEPS[c.toLowerCase()] ?? c);

  const pad = 12;
  const titleH = meta.title ? 22 : 0;
  // legend rows: wrap by estimated text width
  const items = meta.legend ?? [];
  const rows: { x: number; it: LegendItem }[][] = [];
  let x = 0;
  for (const it of items) {
    const iw = 20 + 6.4 * it.name.length + 14;
    if (!rows.length || x + iw > w - 2 * pad) {
      rows.push([]);
      x = 0;
    }
    rows[rows.length - 1].push({ x, it });
    x += iw;
  }
  const legendH = rows.length ? rows.length * 16 + 6 : 0;
  const barH = meta.colorBar ? 34 : 0;
  const W = Math.ceil(w + 2 * pad);
  const H = Math.ceil(pad + titleH + h + legendH + barH + pad);
  const y0 = pad + titleH;

  const parts: string[] = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" font-family="${esc(FONT)}">`,
    `<style>
.plot-grid{stroke:${ink.grid};stroke-width:1}
.plot-axis{stroke:${ink.axis};stroke-width:1}
.plot-ref{stroke:${ink.ref};stroke-width:1;stroke-dasharray:4 3}
.plot-frame{stroke:${ink.grid};fill:none}
.plot-tick{fill:${ink.muted};font-size:10px;font-family:ui-monospace,Menlo,Consolas,monospace}
.plot-label{fill:${ink.muted};font-size:11px}
.plot-dot{stroke:${ink.bg};stroke-width:2}
</style>`,
    `<rect width="100%" height="100%" fill="${ink.bg}"/>`,
  );
  if (meta.title) parts.push(`<text x="${pad}" y="${pad + 13}" font-size="13" font-weight="600" fill="${ink.text}">${esc(meta.title)}</text>`);
  parts.push(`<svg x="${pad}" y="${y0}" width="${w}" height="${h}" overflow="visible">${inner}</svg>`);
  rows.forEach((row, r) => {
    const yy = y0 + h + 6 + r * 16 + 8;
    for (const { x: lx, it } of row) {
      const c = themeColor(it.color, theme);
      parts.push(
        `<line x1="${pad + lx}" x2="${pad + lx + 14}" y1="${yy}" y2="${yy}" stroke="${c}" stroke-width="2"${it.dash ? ' stroke-dasharray="4 3"' : ''}/>`,
        `<text x="${pad + lx + 20}" y="${yy}" dy="0.35em" font-size="11" fill="${ink.text}">${esc(it.name)}</text>`,
      );
    }
  });
  if (meta.colorBar) {
    const cb = meta.colorBar;
    const yy = y0 + h + legendH + 6;
    const id = 'cb';
    parts.push(
      `<defs><linearGradient id="${id}">${cb.stops.map((c, i) => `<stop offset="${(i / Math.max(1, cb.stops.length - 1)) * 100}%" stop-color="${c}"/>`).join('')}</linearGradient></defs>`,
      `<rect x="${pad}" y="${yy}" width="${w}" height="10" rx="3" fill="url(#${id})"/>`,
      `<text x="${pad}" y="${yy + 23}" font-size="10" fill="${ink.muted}" font-family="ui-monospace,Menlo,monospace">${esc(cb.lo)}</text>`,
      `<text x="${pad + w / 2}" y="${yy + 23}" font-size="10" fill="${ink.muted}" text-anchor="middle">${esc(cb.label)}</text>`,
      `<text x="${pad + w}" y="${yy + 23}" font-size="10" fill="${ink.muted}" text-anchor="end" font-family="ui-monospace,Menlo,monospace">${esc(cb.hi)}</text>`,
    );
  }
  parts.push('</svg>');
  return parts.join('\n');
}

/** Rasterize an SVG document at `scale` × its size. */
export async function svgToPng(svgText: string, scale = 2): Promise<Blob> {
  const url = URL.createObjectURL(new Blob([svgText], { type: 'image/svg+xml' }));
  try {
    const img = new Image();
    img.decoding = 'sync';
    await new Promise<void>((ok, fail) => {
      img.onload = () => ok();
      img.onerror = () => fail(new Error('could not render the figure'));
      img.src = url;
    });
    const c = document.createElement('canvas');
    c.width = Math.round(img.width * scale);
    c.height = Math.round(img.height * scale);
    const g = c.getContext('2d')!;
    g.scale(scale, scale);
    g.drawImage(img, 0, 0);
    return await new Promise<Blob>((ok, fail) => c.toBlob((b) => (b ? ok(b) : fail(new Error('PNG encoding failed'))), 'image/png'));
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Rows of cells as CSV (RFC 4180 quoting). */
export function toCsv(rows: (string | number | null | undefined)[][]): string {
  const cell = (v: string | number | null | undefined) => {
    const s = v === null || v === undefined || (typeof v === 'number' && !Number.isFinite(v)) ? '' : String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return rows.map((r) => r.map(cell).join(',')).join('\n') + '\n';
}

/** Rows of cells as tab-separated text (pastes into spreadsheets). */
export function toTsv(rows: (string | number | null | undefined)[][]): string {
  return rows.map((r) => r.map((v) => (v === null || v === undefined || (typeof v === 'number' && !Number.isFinite(v)) ? '' : String(v))).join('\t')).join('\n') + '\n';
}

/** The text cells of an HTML table (header and body), for copying. */
export function tableRows(t: HTMLTableElement): string[][] {
  return Array.from(t.rows).map((r) => Array.from(r.cells).map((c) => (c.textContent ?? '').trim()));
}

export const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 60) || 'figure';

export async function copyPng(png: Promise<Blob>) {
  // Chrome accepts a promise, which keeps the clipboard write inside the user gesture
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
}

export const saveText = (text: string, name: string, type: string) => download(new Blob([text], { type }), name);
