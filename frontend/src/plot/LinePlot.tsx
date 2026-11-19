// Small SVG line/point plot: linear or log axes, crosshair tooltip listing every series,
// line-key legend (two or more series), optional reference lines, and a table view.
import { useEffect, useMemo, useRef, useState } from 'react';
import { FigureMenu } from './FigureMenu';

export interface Series {
  name: string;
  color: string;
  points: [number, number][];
  /** draw point markers (for sparse data) */
  markers?: boolean;
  /** draw no line between points */
  pointsOnly?: boolean;
  /** dashed line (a reference curve, e.g. an approximation) */
  dash?: boolean;
}

export interface RefLine {
  y: number;
  label: string;
}

interface Props {
  series: Series[];
  xLabel: string;
  yLabel: string;
  logX?: boolean;
  logY?: boolean;
  height?: number;
  xDomain?: [number, number];
  yDomain?: [number, number];
  refLines?: RefLine[];
  formatX?: (x: number) => string;
  formatY?: (y: number) => string;
  /** shown above the plot (screen readers get it as the SVG title) */
  title?: string;
  /** figure title and file name for exports (default: the title, else the y label) */
  name?: string;
}

const M = { l: 52, r: 12, t: 10, b: 34 };

function niceStep(span: number, n: number) {
  const raw = span / Math.max(1, n);
  const p = 10 ** Math.floor(Math.log10(raw));
  const f = raw / p;
  return (f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10) * p;
}

export function linTicks([a, b]: [number, number], n: number) {
  const s = niceStep(b - a, n);
  const out: number[] = [];
  // a step below the floating-point resolution of the values would never advance
  if (!(s > 0) || !Number.isFinite(s) || a + s === a) return [a];
  for (let v = Math.ceil(a / s) * s; v <= b + s * 1e-9 && out.length < 200; v += s) out.push(Math.abs(v) < s * 1e-9 ? 0 : v);
  return out;
}

function logTicks([a, b]: [number, number]) {
  const out: number[] = [];
  const e0 = Math.floor(Math.log10(a));
  const e1 = Math.ceil(Math.log10(b));
  const step = Math.max(1, Math.ceil((e1 - e0) / 6));
  for (let e = e0; e <= e1; e += step) {
    const v = 10 ** e;
    if (v >= a * 0.999 && v <= b * 1.001) out.push(v);
  }
  return out;
}

export const fmtSi = (v: number) => {
  if (v === 0) return '0';
  const a = Math.abs(v);
  if (a >= 1e4 || a < 1e-2) {
    const e = Math.floor(Math.log10(a));
    const m = v / 10 ** e;
    return Math.abs(m - 1) < 1e-9 ? `1e${e}` : `${Number(m.toPrecision(2))}e${e}`;
  }
  return String(Number(v.toPrecision(3)));
};

function domain(vals: number[], log: boolean, pad: boolean): [number, number] {
  const v = vals.filter((x) => Number.isFinite(x) && (!log || x > 0));
  if (!v.length) return log ? [1e-3, 1] : [0, 1];
  let lo = Math.min(...v);
  let hi = Math.max(...v);
  if (log) {
    if (lo === hi) [lo, hi] = [lo / 10, hi * 10];
    return [10 ** Math.floor(Math.log10(lo)), 10 ** Math.ceil(Math.log10(hi))];
  }
  if (lo === hi) [lo, hi] = [lo - (Math.abs(lo) || 1) * 0.1, hi + (Math.abs(hi) || 1) * 0.1];
  if (pad) {
    const p = (hi - lo) * 0.08;
    [lo, hi] = [lo - p, hi + p];
  }
  return [lo, hi];
}

export function LinePlot(p: Props) {
  const host = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const [width, setWidth] = useState(360);
  const [hover, setHover] = useState<number | null>(null);
  const [table, setTable] = useState(false);
  const height = p.height ?? 180;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(200, el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const geo = useMemo(() => {
    const xs = p.series.flatMap((s) => s.points.map((q) => q[0]));
    const ys = [...p.series.flatMap((s) => s.points.map((q) => q[1])), ...(p.refLines ?? []).map((r) => r.y)];
    const xd = p.xDomain ?? domain(xs, !!p.logX, false);
    const yd = p.yDomain ?? domain(ys, !!p.logY, true);
    const tx = (v: number) => (p.logX ? Math.log10(v) : v);
    const ty = (v: number) => (p.logY ? Math.log10(v) : v);
    const [x0, x1] = [tx(xd[0]), tx(xd[1])];
    const [y0, y1] = [ty(yd[0]), ty(yd[1])];
    const W = width - M.l - M.r;
    const H = height - M.t - M.b;
    const sx = (v: number) => M.l + ((tx(v) - x0) / (x1 - x0 || 1)) * W;
    const sy = (v: number) => M.t + H - ((ty(v) - y0) / (y1 - y0 || 1)) * H;
    const xt = p.logX ? logTicks(xd) : linTicks(xd, Math.max(2, Math.floor(W / 70)));
    const yt = p.logY ? logTicks(yd) : linTicks(yd, Math.max(2, Math.floor(H / 36)));
    const allX = [...new Set(xs)].sort((a, b) => a - b);
    return { sx, sy, xt, yt, W, H, allX, xd, yd };
  }, [p.series, p.refLines, p.xDomain, p.yDomain, p.logX, p.logY, width, height]);

  const fx = p.formatX ?? fmtSi;
  const fy = p.formatY ?? fmtSi;
  const inY = (v: number) => (!p.logY || v > 0) && v >= geo.yd[0] * (p.logY ? 0.999 : 1) - 1e-300 && v <= geo.yd[1] * 1.001 + 1e-300;
  // a missing value (NaN, or ≤ 0 on a log axis) breaks the line: a gap, not a bridge
  const path = (s: Series) => {
    let d = '';
    let pen = false;
    for (const q of s.points) {
      if (!Number.isFinite(q[1]) || (p.logY && q[1] <= 0)) {
        pen = false;
        continue;
      }
      d += `${pen ? 'L' : 'M'}${geo.sx(q[0]).toFixed(1)},${geo.sy(q[1]).toFixed(1)}`;
      pen = true;
    }
    return d;
  };

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    const r = (e.currentTarget as SVGRectElement).getBoundingClientRect();
    const px = e.clientX - r.left + M.l;
    let best: number | null = null;
    let bd = Infinity;
    for (const x of geo.allX) {
      const d = Math.abs(geo.sx(x) - px);
      if (d < bd) [bd, best] = [d, x];
    }
    setHover(best);
  };

  const at = (s: Series, x: number) => s.points.find((q) => q[0] === x)?.[1];
  const hx = hover !== null ? geo.sx(hover) : 0;

  return (
    <div className="plot" ref={host}>
      {p.title && <div className="plot-title">{p.title}</div>}
      <div className="plot-area">
        <svg ref={svgRef} width={width} height={height} role="img" aria-label={p.title ?? p.yLabel}>
          {/* grid and axes */}
          {geo.yt.map((v) => (
            <g key={`y${v}`}>
              <line className="plot-grid" x1={M.l} x2={M.l + geo.W} y1={geo.sy(v)} y2={geo.sy(v)} />
              <text className="plot-tick" x={M.l - 6} y={geo.sy(v)} dy="0.32em" textAnchor="end">
                {fy(v)}
              </text>
            </g>
          ))}
          {geo.xt.map((v) => (
            <text key={`x${v}`} className="plot-tick" x={geo.sx(v)} y={M.t + geo.H + 14} textAnchor="middle">
              {fx(v)}
            </text>
          ))}
          <line className="plot-axis" x1={M.l} x2={M.l + geo.W} y1={M.t + geo.H} y2={M.t + geo.H} />
          <text className="plot-label" x={M.l + geo.W / 2} y={height - 4} textAnchor="middle">
            {p.xLabel}
          </text>
          <text className="plot-label" transform={`translate(11 ${M.t + geo.H / 2}) rotate(-90)`} textAnchor="middle">
            {p.yLabel}
          </text>
          {(p.refLines ?? []).filter((r) => inY(r.y)).map((r) => (
            <g key={r.label}>
              <line className="plot-ref" x1={M.l} x2={M.l + geo.W} y1={geo.sy(r.y)} y2={geo.sy(r.y)} />
              <text className="plot-tick" x={M.l + geo.W - 2} y={geo.sy(r.y) - 4} textAnchor="end">
                {r.label}
              </text>
            </g>
          ))}
          {/* data */}
          {p.series.map((s) => (
            <g key={s.name}>
              {!s.pointsOnly && <path d={path(s)} fill="none" stroke={s.color} strokeWidth={s.dash ? 1.5 : 2} strokeDasharray={s.dash ? '5 4' : undefined} strokeLinejoin="round" strokeLinecap="round" />}
              {(s.markers || s.pointsOnly) &&
                s.points
                  .filter((q) => Number.isFinite(q[1]) && (!p.logY || q[1] > 0))
                  .map((q, i) => <circle key={i} className="plot-dot" cx={geo.sx(q[0])} cy={geo.sy(q[1])} r={4} fill={s.color} />)}
            </g>
          ))}
          {hover !== null && (
            <g pointerEvents="none" data-export="skip">
              <line className="plot-cross" x1={hx} x2={hx} y1={M.t} y2={M.t + geo.H} />
              {p.series.map((s) => {
                const v = at(s, hover);
                return v !== undefined && Number.isFinite(v) && (!p.logY || v > 0) ? (
                  <circle key={s.name} className="plot-dot" cx={hx} cy={geo.sy(v)} r={4} fill={s.color} />
                ) : null;
              })}
            </g>
          )}
          <rect
            data-export="skip"
            x={M.l}
            y={M.t}
            width={geo.W}
            height={geo.H}
            fill="transparent"
            onPointerMove={onMove}
            onPointerLeave={() => setHover(null)}
          />
        </svg>
        {hover !== null && (
          <div className="plot-tip" style={{ left: Math.min(hx + 10, width - 150), top: M.t }}>
            <div className="plot-tip-x">
              {p.xLabel}: {fx(hover)}
            </div>
            {p.series.map((s) => {
              const v = at(s, hover);
              return v === undefined ? null : (
                <div key={s.name} className="plot-tip-row">
                  <span className="plot-key" style={{ background: s.color }} />
                  <b>{fy(v)}</b>
                  <span>{s.name}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      <div className="plot-foot">
        {p.series.length > 1 && (
          <div className="plot-legend">
            {p.series.map((s) => (
              <span key={s.name}>
                <span className={`plot-key ${s.dash ? 'dash' : ''}`} style={{ background: s.dash ? undefined : s.color, borderColor: s.color }} />
                {s.name}
              </span>
            ))}
          </div>
        )}
        <button className="plot-table-toggle" onClick={() => setTable(!table)}>
          {table ? 'hide table' : 'table'}
        </button>
        <FigureMenu
          name={p.name ?? p.title ?? p.yLabel}
          svg={() => svgRef.current}
          meta={{
            title: p.name ?? p.title,
            legend: p.series.length > 1 ? p.series.map((s) => ({ name: s.name, color: s.color, dash: s.dash })) : undefined,
          }}
          rows={() => [[p.xLabel, ...p.series.map((s) => s.name)], ...geo.allX.map((x) => [x, ...p.series.map((s) => at(s, x))])]}
        />
      </div>
      {table && (
        <div className="plot-table">
          <table className="bodies compact">
            <thead>
              <tr>
                <th>{p.xLabel}</th>
                {p.series.map((s) => (
                  <th key={s.name}>{s.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {geo.allX.map((x) => (
                <tr key={x}>
                  <td className="num">{fx(x)}</td>
                  {p.series.map((s) => {
                    const v = at(s, x);
                    return (
                      <td key={s.name} className="num">
                        {v === undefined ? '' : fy(v)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
