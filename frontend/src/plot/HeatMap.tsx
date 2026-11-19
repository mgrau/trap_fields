// Square 2D map of a scalar on an n×n grid (row-major [v][u], v up), drawn from a
// precomputed RGBA image, with axes, point markers and a hover readout.
import { useEffect, useMemo, useRef, useState } from 'react';
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

interface Props {
  n: number;
  /** grid nodes at −half + k·2half/(n−1), k = 0…n−1 (axis units, both axes); each pixel is
   * centered on its node, so the image spans half a step more on every side */
  half: number;
  /** RGBA, row j = v index j (bottom to top) */
  rgba: Uint8Array;
  uLabel: string;
  vLabel: string;
  markers?: MapMarker[];
  /** hover text for grid cell (i along u, j along v) */
  readout?: (i: number, j: number) => string | null;
  /** largest drawn side (px) */
  maxSize?: number;
}

const M = { l: 48, r: 10, t: 8, b: 34 };

export function HeatMap(p: Props) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(400);
  const [hover, setHover] = useState<{ i: number; j: number; x: number; y: number } | null>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(220, el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // the image, with v increasing upward
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    c.width = p.n;
    c.height = p.n;
    const img = new ImageData(p.n, p.n);
    for (let j = 0; j < p.n; j++) img.data.set(p.rgba.subarray(4 * p.n * j, 4 * p.n * (j + 1)), 4 * p.n * (p.n - 1 - j));
    c.getContext('2d')!.putImageData(img, 0, 0);
  }, [p.rgba, p.n]);

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
        <canvas ref={canvas} className="heatmap-img" style={{ left: M.l, top: M.t, width: side, height: side }} />
        <svg width={side + M.l + M.r} height={side + M.t + M.b} role="img" aria-label={`${p.vLabel} against ${p.uLabel}`}>
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
                stroke={m.ring ? m.color : '#1c2027'}
                strokeWidth={m.ring ? 2 : 1.5}
              >
                <title>{m.label}</title>
              </circle>
            ))}
          {hover && (
            <g pointerEvents="none">
              <line className="plot-cross" x1={geo.sx(cell(hover.i))} x2={geo.sx(cell(hover.i))} y1={M.t} y2={M.t + side} />
              <line className="plot-cross" x1={M.l} x2={M.l + side} y1={geo.sy(cell(hover.j))} y2={geo.sy(cell(hover.j))} />
            </g>
          )}
          <rect x={M.l} y={M.t} width={side} height={side} fill="transparent" onPointerMove={onMove} onPointerLeave={() => setHover(null)} />
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
    </div>
  );
}
