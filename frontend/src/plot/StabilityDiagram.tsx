// Mathieu stability diagram (a against |q|) with the trap's operating points: per RF
// quadrupole axis the equation u'' + (a − 2q cos 2τ)u = 0 is stable between the
// characteristic curves a_0(q) and b_1(q) (first region; the second, a_1 to b_2, is drawn
// when in view). Iso-β lines (secular frequency ω = βΩ/2) cover the first region.
import { useEffect, useMemo, useRef, useState } from 'react';
import { betaOf, charValue, isoBetaA, region } from '../physics/mathieu';
import { FigureMenu } from './FigureMenu';
import { fmtSi, linTicks } from './LinePlot';

export interface OperatingPoint {
  label: string;
  color: string;
  q: number;
  a: number;
}

export interface Path {
  label: string;
  color: string;
  /** (|q|, a) along a sweep */
  points: [number, number][];
}

interface Props {
  points: OperatingPoint[];
  paths?: Path[];
  /** RF frequency Ω/2π (Hz), for secular frequencies in the readout */
  rfFreq?: number;
  view: 'full' | 'zoom';
  height?: number;
  name: string;
}

const M = { l: 58, r: 14, t: 10, b: 36 };
const N = 220;

export function StabilityDiagram(p: Props) {
  const host = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const [width, setWidth] = useState(420);
  const [hover, setHover] = useState<{ q: number; a: number; x: number; y: number } | null>(null);
  const height = p.height ?? 300;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(240, el.clientWidth)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // domains
  const dom = useMemo(() => {
    const all: [number, number][] = [...p.points.map((x) => [Math.abs(x.q), x.a] as [number, number]), ...(p.paths ?? []).flatMap((x) => x.points)];
    const qmax = Math.max(0, ...all.map((x) => x[0]).filter(Number.isFinite));
    if (p.view === 'full') {
      const qhi = Math.max(1, qmax * 1.15);
      return { q: [0, qhi] as [number, number], a: [Math.min(-0.6, charValue('a', 0, qhi) * 1.05), 1.05] as [number, number] };
    }
    const qhi = Math.max(1e-3, qmax * 1.6);
    const as = all.map((x) => x[1]).filter(Number.isFinite);
    const floor = charValue('a', 0, qhi);
    let lo = Math.min(floor, ...as);
    let hi = Math.max(...as, 0);
    const pad = 0.12 * (hi - lo || Math.abs(lo) || 1e-3);
    lo -= pad;
    hi = Math.max(hi + pad, 0.25 * Math.abs(lo));
    return { q: [0, qhi] as [number, number], a: [lo, hi] as [number, number] };
  }, [p.points, p.paths, p.view]);

  const W = width - M.l - M.r;
  const H = height - M.t - M.b;
  const sx = (q: number) => M.l + ((q - dom.q[0]) / (dom.q[1] - dom.q[0])) * W;
  const sy = (a: number) => M.t + H - ((a - dom.a[0]) / (dom.a[1] - dom.a[0])) * H;
  const clampY = (a: number) => Math.min(Math.max(a, dom.a[0] - (dom.a[1] - dom.a[0])), dom.a[1] + (dom.a[1] - dom.a[0]));

  // boundary curves and iso-β lines, sampled over the q range in view
  const curves = useMemo(() => {
    const qs = Array.from({ length: N + 1 }, (_, i) => dom.q[0] + ((dom.q[1] - dom.q[0]) * i) / N);
    const a0 = qs.map((q) => charValue('a', 0, q));
    const b1 = qs.map((q) => charValue('b', 1, q));
    const a1 = qs.map((q) => charValue('a', 1, q));
    const b2 = qs.map((q) => charValue('b', 2, q));
    const betas = p.view === 'full' ? [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] : [];
    const iso = betas.map((b) => ({ beta: b, a: qs.map((q) => isoBetaA(b, q)) }));
    return { qs, a0, b1, a1, b2, iso };
  }, [dom, p.view]);

  const line = (ys: number[]) => curves.qs.map((q, i) => `${i ? 'L' : 'M'}${sx(q).toFixed(1)},${sy(clampY(ys[i])).toFixed(1)}`).join('');
  const band = (lo: number[], hi: number[]) =>
    curves.qs.map((q, i) => `${i ? 'L' : 'M'}${sx(q).toFixed(1)},${sy(clampY(lo[i])).toFixed(1)}`).join('') +
    curves.qs
      .map((_, i) => curves.qs.length - 1 - i)
      .map((i) => `L${sx(curves.qs[i]).toFixed(1)},${sy(clampY(hi[i])).toFixed(1)}`)
      .join('') +
    'Z';
  const secondInView = curves.a1.some((a) => a < dom.a[1]);
  const qt = linTicks(dom.q, Math.max(2, Math.floor(W / 70)));
  const at = linTicks(dom.a, Math.max(2, Math.floor(H / 36)));

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const fx = (e.clientX - r.left) / r.width;
    const fy = 1 - (e.clientY - r.top) / r.height;
    setHover({ q: dom.q[0] + fx * (dom.q[1] - dom.q[0]), a: dom.a[0] + fy * (dom.a[1] - dom.a[0]), x: e.clientX - r.left + M.l, y: e.clientY - r.top + M.t });
  };
  const describe = (a: number, q: number) => {
    const reg = region(a, q);
    const b = betaOf(a, q);
    const f = b !== null && p.rfFreq ? ` · ω/2π ≈ ${fmtFreq((b * p.rfFreq) / 2)}` : '';
    return reg === 0 ? 'unstable' : reg === 1 ? `stable, β = ${b?.toPrecision(3)}${f}` : `stable (region ${reg})`;
  };
  const clipId = useMemo(() => `stab-clip-${Math.random().toString(36).slice(2)}`, []);
  const ink = 'var(--muted)';

  return (
    <div className="plot" ref={host}>
      <div className="plot-area">
        <svg ref={svgRef} width={width} height={height} role="img" aria-label="Mathieu stability diagram, a against q">
          <defs>
            <clipPath id={clipId}>
              <rect x={M.l} y={M.t} width={W} height={H} />
            </clipPath>
          </defs>
          {at.map((v) => (
            <g key={`a${v}`}>
              <line className="plot-grid" x1={M.l} x2={M.l + W} y1={sy(v)} y2={sy(v)} />
              <text className="plot-tick" x={M.l - 6} y={sy(v)} dy="0.32em" textAnchor="end">
                {fmtSi(v)}
              </text>
            </g>
          ))}
          {qt.map((v) => (
            <text key={`q${v}`} className="plot-tick" x={sx(v)} y={M.t + H + 14} textAnchor="middle">
              {fmtSi(v)}
            </text>
          ))}
          <text className="plot-label" x={M.l + W / 2} y={height - 4} textAnchor="middle">
            |q|
          </text>
          <text className="plot-label" transform={`translate(12 ${M.t + H / 2}) rotate(-90)`} textAnchor="middle">
            a
          </text>
          <g clipPath={`url(#${clipId})`}>
            <path d={band(curves.a0, curves.b1)} fill="#3987e5" fillOpacity={0.16} stroke="none" />
            {secondInView && <path d={band(curves.a1, curves.b2)} fill="#3987e5" fillOpacity={0.1} stroke="none" />}
            {curves.iso.map((c) => (
              <path key={c.beta} d={line(c.a)} fill="none" stroke="#6b7280" strokeWidth={1} strokeDasharray="3 3" />
            ))}
            <path d={line(curves.a0)} fill="none" stroke="#8b93a1" strokeWidth={1.5} />
            <path d={line(curves.b1)} fill="none" stroke="#8b93a1" strokeWidth={1.5} />
            {secondInView && <path d={line(curves.a1)} fill="none" stroke="#8b93a1" strokeWidth={1.5} />}
            {secondInView && <path d={line(curves.b2)} fill="none" stroke="#8b93a1" strokeWidth={1.5} />}
            {dom.a[0] < 0 && dom.a[1] > 0 && <line className="plot-axis" x1={M.l} x2={M.l + W} y1={sy(0)} y2={sy(0)} />}
            {(p.paths ?? []).map((path) => (
              <g key={path.label}>
                <path
                  d={path.points
                    .map((x, i) => {
                      if (!Number.isFinite(x[0]) || !Number.isFinite(x[1])) return '';
                      const prev = path.points[i - 1];
                      const pen = i > 0 && Number.isFinite(prev[0]) && Number.isFinite(prev[1]);
                      return `${pen ? 'L' : 'M'}${sx(x[0]).toFixed(1)},${sy(x[1]).toFixed(1)}`;
                    })
                    .join('')}
                  fill="none"
                  stroke={path.color}
                  strokeWidth={1.5}
                  strokeOpacity={0.7}
                />
                {path.points.map((x, i) => (Number.isFinite(x[1]) ? <circle key={i} cx={sx(x[0])} cy={sy(x[1])} r={2} fill={path.color} fillOpacity={0.7} /> : null))}
              </g>
            ))}
            {/* decreasing radii: points at the same (|q|, a), like the two radial axes, stay visible as rings */}
            {p.points.map((x, i) => (
              <circle key={x.label} className="plot-dot" cx={sx(Math.abs(x.q))} cy={sy(x.a)} r={7 - 1.8 * i} fill={x.color}>
                <title>{`${x.label}: q = ${x.q.toPrecision(3)}, a = ${x.a.toPrecision(3)} — ${describe(x.a, x.q)}`}</title>
              </circle>
            ))}
          </g>
          {/* labels where the iso-β lines are well apart: near q = 0, where a = β² */}
          {curves.iso
            .filter((c) => [0.2, 0.4, 0.6, 0.8].includes(c.beta))
            .map((c) => {
              const k = Math.round(0.04 * N);
              const y = sy(c.a[k]);
              return y > M.t + 6 && y < M.t + H - 2 ? (
                <text key={c.beta} className="plot-tick" x={sx(curves.qs[k]) + 3} y={y - 3}>
                  β = {c.beta}
                </text>
              ) : null;
            })}
          {p.view === 'full' && (
            <text className="plot-label" x={sx(dom.q[1] * 0.42)} y={sy(0.3)} fill={ink}>
              stable
            </text>
          )}
          {hover && (
            <g pointerEvents="none" data-export="skip">
              <line className="plot-cross" x1={hover.x} x2={hover.x} y1={M.t} y2={M.t + H} />
              <line className="plot-cross" x1={M.l} x2={M.l + W} y1={hover.y} y2={hover.y} />
            </g>
          )}
          <rect data-export="skip" x={M.l} y={M.t} width={W} height={H} fill="transparent" onPointerMove={onMove} onPointerLeave={() => setHover(null)} />
        </svg>
        {hover && (
          <div className="plot-tip" style={{ left: Math.min(hover.x + 12, width - 230), top: Math.max(M.t, hover.y - 52) }}>
            <div className="plot-tip-x">
              |q| = {fmtSi(hover.q)}, a = {fmtSi(hover.a)}
            </div>
            <div className="plot-tip-row">{describe(hover.a, hover.q)}</div>
          </div>
        )}
      </div>
      <div className="plot-foot">
        <div className="plot-legend">
          {p.points.map((x) => (
            <span key={x.label}>
              <span className="plot-key" style={{ background: x.color }} />
              {x.label}
            </span>
          ))}
        </div>
        <FigureMenu
          name={p.name}
          svg={() => svgRef.current}
          meta={{ title: p.name, legend: p.points.map((x) => ({ name: x.label, color: x.color })) }}
          rows={() => [
            ['axis', 'q', 'a', 'beta', 'secular frequency (Hz, Mathieu)'],
            ...p.points.map((x) => {
              const b = betaOf(x.a, x.q);
              return [x.label, x.q, x.a, b, b !== null && p.rfFreq ? (b * p.rfFreq) / 2 : null];
            }),
          ]}
        />
      </div>
    </div>
  );
}

function fmtFreq(hz: number) {
  return hz >= 1e6 ? `${(hz / 1e6).toFixed(3)} MHz` : `${(hz / 1e3).toFixed(1)} kHz`;
}
