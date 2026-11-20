// Square 2D map of a scalar on an n×n grid (row-major [v][u], v up), drawn from a
// precomputed RGBA image, with axes, point markers, direction lines and a hover readout.
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ColorBar, LegendItem } from './exportFigure';
import { FigureMenu } from './FigureMenu';
import { fmtSi, linTicks } from './LinePlot';

export interface MapMarker {
  /** position in axis units */
  u: number;
  v: number;
  color: string;
  label: string;
  /** radius in px */
  r?: number;
  ring?: boolean;
}

/** A line through (u, v) along (du, dv), across the whole map, labeled at its end. */
export interface MapLine {
  u: number;
  v: number;
  du: number;
  dv: number;
  color: string;
  label: string;
  dash?: boolean;
}

interface Props {
  n: number;
  /** grid nodes at −half + k·2half/(n−1), k = 0…n−1 (axis units, both axes); each pixel is
   * centered on its node, so the image spans half a step more on every side */
  half: number;
  /** RGBA, row j = v index j (bottom to top); `imageSize` per side (default n), spanning
   * the same extent as one pixel per node, so it can be resampled finer than the grid */
  rgba: Uint8Array;
  imageSize?: number;
  uLabel: string;
  vLabel: string;
  markers?: MapMarker[];
  lines?: MapLine[];
  /** keys shown under the map and in exported figures */
  legend?: LegendItem[];
  /** hover text for grid cell (i along u, j along v) */
  readout?: (i: number, j: number) => string | null;
  /** largest drawn side (px) */
  maxSize?: number;
  /** the mapped values (row-major like rgba) and their name, for data export */
  values?: (number | null)[];
  valueLabel?: string;
  /** figure title / file name, and the color bar, for exports */
  name: string;
  colorBar?: ColorBar;
}

const M = { l: 48, r: 10, t: 8, b: 34 };

export function HeatMap(p: Props) {
  const host = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const [width, setWidth] = useState(400);
  const [hover, setHover] = useState<{ i: number; j: number; x: number; y: number } | null>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(220, el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // the image as a data URL, with v increasing upward (an SVG <image>, so it exports with the figure)
  const size = p.imageSize ?? p.n;
  const href = useMemo(() => {
    const c = document.createElement('canvas');
    c.width = size;
    c.height = size;
    const img = new ImageData(size, size);
    for (let j = 0; j < size; j++) img.data.set(p.rgba.subarray(4 * size * j, 4 * size * (j + 1)), 4 * size * (size - 1 - j));
    c.getContext('2d')!.putImageData(img, 0, 0);
    return c.toDataURL('image/png');
  }, [p.rgba, size]);
  const clipId = `heatmap-clip-${useId().replace(/:/g, '')}`;

  const side = Math.min(p.maxSize ?? 560, width - M.l - M.r);
  const ext = p.n > 1 ? (p.half * p.n) / (p.n - 1) : p.half;
  const geo = useMemo(() => {
    const s = (x: number) => ((x + ext) / (2 * ext)) * side;
    const ticks = linTicks([-p.half, p.half], Math.max(2, Math.floor(side / 70)));
    return { sx: (u: number) => M.l + s(u), sy: (v: number) => M.t + side - s(v), ticks };
  }, [p.half, ext, side]);

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const fx = (e.clientX - r.left) / r.width;
    const fy = 1 - (e.clientY - r.top) / r.height;
    const i = Math.min(p.n - 1, Math.max(0, Math.floor(fx * p.n)));
    const j = Math.min(p.n - 1, Math.max(0, Math.floor(fy * p.n)));
    setHover({ i, j, x: e.clientX - r.left + M.l, y: e.clientY - r.top + M.t });
  };
  // node k (the center of pixel k)
  const cell = (k: number) => (p.n > 1 ? -p.half + (k * 2 * p.half) / (p.n - 1) : 0);
  const text = hover && p.readout ? p.readout(hover.i, hover.j) : null;

  return (
    <div className="plot heatmap" ref={host}>
      <div className="plot-area" style={{ width: side + M.l + M.r, height: side + M.t + M.b }}>
        <svg ref={svgRef} width={side + M.l + M.r} height={side + M.t + M.b} role="img" aria-label={`${p.vLabel} against ${p.uLabel}`}>
          <image href={href} x={M.l} y={M.t} width={side} height={side} preserveAspectRatio="none" style={{ imageRendering: size > p.n ? 'auto' : 'pixelated' }} />
          <clipPath id={clipId}>
            <rect x={M.l} y={M.t} width={side} height={side} />
          </clipPath>
          {(p.lines ?? []).map((l, k) => {
            // through (u, v) to both edges of the map; the label sits inside the far end
            const n = Math.hypot(l.du, l.dv);
            if (!(n > 0)) return null;
            const [du, dv] = [l.du / n, l.dv / n];
            // parameter where the line meets the edge u = ±ext (or v = ±ext), forward and back
            const hit = (s: number, c: number, d: number) => (d ? (s * Math.sign(d) * ext - c) / d : s * Infinity);
            const t1 = Math.min(hit(1, l.u, du), hit(1, l.v, dv));
            const t0 = Math.max(hit(-1, l.u, du), hit(-1, l.v, dv));
            if (!(t1 > t0)) return null;
            const [x0, y0, x1, y1] = [geo.sx(l.u + t0 * du), geo.sy(l.v + t0 * dv), geo.sx(l.u + t1 * du), geo.sy(l.v + t1 * dv)];
            const len = Math.hypot(x1 - x0, y1 - y0);
            // label beside the line near its far end, off the line: above a flat line,
            // to the right of a steep one
            const [ex, ey] = [(x1 - x0) / len, (y1 - y0) / len];
            let [nx, ny] = [-ey, ex];
            if (Math.abs(ny) > 0.5 ? ny > 0 : nx < 0) [nx, ny] = [-nx, -ny];
            const back = Math.min(8 + 8 * Math.abs(ey), len / 2);
            const [lx, ly] = [x1 - ex * back + nx * 9, y1 - ey * back + ny * 9];
            const anchor = Math.abs(nx) > 0.5 ? (nx > 0 ? 'start' : 'end') : ex > 0 ? 'end' : 'start';
            return (
              <g key={k} className="map-line">
                <g clipPath={`url(#${clipId})`}>
                  <line x1={x0} y1={y0} x2={x1} y2={y1} stroke="#ffffff" strokeOpacity={0.75} strokeWidth={4} />
                  <line x1={x0} y1={y0} x2={x1} y2={y1} stroke={l.color} strokeWidth={2} strokeDasharray={l.dash ? '6 4' : undefined} />
                </g>
                <text x={lx} y={ly} dy={ny > 0.5 ? '0.8em' : Math.abs(ny) > 0.5 ? '-0.1em' : '0.32em'} textAnchor={anchor} fill={l.color} stroke="#ffffff" strokeWidth={3} paintOrder="stroke" className="map-line-label">
                  {l.label}
                </text>
              </g>
            );
          })}
          <rect className="plot-frame" x={M.l} y={M.t} width={side} height={side} fill="none" />
          {geo.ticks.map((t) => (
            <g key={t}>
              <text className="plot-tick" x={geo.sx(t)} y={M.t + side + 14} textAnchor="middle">
                {fmtSi(t)}
              </text>
              <text className="plot-tick" x={M.l - 6} y={geo.sy(t)} dy="0.32em" textAnchor="end">
                {fmtSi(t)}
              </text>
            </g>
          ))}
          <text className="plot-label" x={M.l + side / 2} y={M.t + side + M.b - 4} textAnchor="middle">
            {p.uLabel}
          </text>
          <text className="plot-label" transform={`translate(11 ${M.t + side / 2}) rotate(-90)`} textAnchor="middle">
            {p.vLabel}
          </text>
          {(p.markers ?? [])
            .filter((m) => Math.abs(m.u) <= ext && Math.abs(m.v) <= ext)
            .map((m, k) => (
              <circle
                key={k}
                cx={geo.sx(m.u)}
                cy={geo.sy(m.v)}
                r={m.r ?? 4}
                fill={m.ring ? 'none' : m.color}
                stroke={m.ring ? m.color : '#ffffff'}
                strokeWidth={m.ring ? 2 : 1.5}
              >
                <title>{m.label}</title>
              </circle>
            ))}
          {hover && (
            <g pointerEvents="none" data-export="skip">
              <line className="plot-cross" x1={geo.sx(cell(hover.i))} x2={geo.sx(cell(hover.i))} y1={M.t} y2={M.t + side} />
              <line className="plot-cross" x1={M.l} x2={M.l + side} y1={geo.sy(cell(hover.j))} y2={geo.sy(cell(hover.j))} />
            </g>
          )}
          <rect data-export="skip" x={M.l} y={M.t} width={side} height={side} fill="transparent" onPointerMove={onMove} onPointerLeave={() => setHover(null)} />
        </svg>
        {hover && (
          <div className="plot-tip" style={{ left: Math.min(hover.x + 12, side + M.l - 170), top: Math.max(M.t, hover.y - 48) }}>
            <div className="plot-tip-x">
              {p.uLabel.split(' ')[0]} {fmtSi(cell(hover.i))}, {p.vLabel.split(' ')[0]} {fmtSi(cell(hover.j))}
            </div>
            {text && <div className="plot-tip-row">{text}</div>}
          </div>
        )}
      </div>
      <div className="plot-foot">
        <div className="plot-legend">
          {(p.legend ?? []).map((it) => (
            <span key={it.name}>
              <span className={`plot-key ${it.dash ? 'dash' : ''}`} style={{ background: it.dash ? undefined : it.color, borderColor: it.color }} />
              {it.name}
            </span>
          ))}
        </div>
        <FigureMenu
          name={p.name}
          svg={() => svgRef.current}
          meta={{ title: p.name, colorBar: p.colorBar, legend: p.legend }}
          rows={() => {
            const vals = p.values ?? [];
            const axis = Array.from({ length: p.n }, (_, k) => Number(cell(k).toPrecision(6)));
            return [
              [`${p.vLabel} \\ ${p.uLabel}${p.valueLabel ? `: ${p.valueLabel}` : ''}`, ...axis],
              ...axis.map((v, j) => [v, ...axis.map((_, i) => vals[j * p.n + i] ?? null)]).reverse(),
            ];
          }}
        />
      </div>
    </div>
  );
}
