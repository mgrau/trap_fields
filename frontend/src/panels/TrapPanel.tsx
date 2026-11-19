import { useEffect, useMemo, useRef, useState } from 'react';
import { api, type AppStateDto, type SolveSummary, type TrapParams, type TrapResult, type Vec3 } from '../api';
import { MODE_COLORS, ROLE_COLORS, ROLE_NAMES, sliceRgba, viridisCss } from '../colors';
import { HeatMap } from '../plot/HeatMap';
import { StabilityDiagram, type OperatingPoint, type Path } from '../plot/StabilityDiagram';
import { LinePlot, type Series } from '../plot/LinePlot';
import { sliceScale } from '../results';
import { useStore, type SliceState } from '../store';
import { len, vec } from '../units';
import { Card, KV, LengthField, NumberField, Section, Seg } from '../widgets';
import { sig } from './SolvePanel';
import { ChainCards, ChainSection, SweepCards, SweepSection } from './TrapExtras';

const IONS: [string, number][] = [
  ['⁹Be⁺', 9.01218],
  ['²⁴Mg⁺', 23.98504],
  ['²⁵Mg⁺', 24.98584],
  ['⁴⁰Ca⁺', 39.96259],
  ['⁴³Ca⁺', 42.95877],
  ['⁸⁸Sr⁺', 87.90561],
  ['¹³⁷Ba⁺', 136.90583],
  ['¹³⁸Ba⁺', 137.90525],
  ['¹⁷¹Yb⁺', 170.93633],
  ['¹⁷⁴Yb⁺', 173.93887],
];

const EV_K = 11604.518; // kelvin per eV

export const ION_COLOR = '#ff4fd8';
const AMU = 1.66053906660e-27;
const E_CHARGE = 1.602176634e-19;

export const fmtFreq = (hz: number | null) => (hz === null ? '—' : hz >= 1e6 ? `${(hz / 1e6).toFixed(4)} MHz` : `${(hz / 1e3).toFixed(2)} kHz`);
const fmtLen = (m: number) => (m > 1 ? '> 1 m' : m >= 1e-3 ? `${(m * 1e3).toPrecision(3)} mm` : `${(m * 1e6).toPrecision(3)} µm`);
export const fmtEv = (ev: number) => (Math.abs(ev) >= 0.1 ? `${ev.toFixed(3)} eV` : `${(ev * 1e3).toFixed(2)} meV`);

/** Current role of each electrode, from the body labels. */
function electrodeRoles(app: AppStateDto): Record<string, string> {
  const r: Record<string, string> = {};
  for (const b of app.bodies) if (b.label) r[b.label.electrode] ??= b.label.role;
  return r;
}

/** Fill in voltages for electrodes missing from `p`: RF electrodes get `vrf`, the rest 0 V. */
function withDefaults(p: TrapParams | null, sol: SolveSummary, roles: Record<string, string>, vrf = 100): TrapParams {
  const base: TrapParams = p ?? { ion: { mass_amu: 39.96259, charge_e: 1 }, rf_freq: 30e6, rf: {}, dc: {}, depth: false };
  const rf = { ...base.rf };
  const dc = { ...base.dc };
  for (const e of sol.electrodes) {
    if (e.floating) continue;
    rf[e.name] ??= roles[e.name] === 'rf' ? vrf : 0;
    dc[e.name] ??= 0;
  }
  // drop electrodes that no longer exist or became floating
  const keep = new Set(sol.electrodes.filter((e) => !e.floating).map((e) => e.name));
  for (const k of Object.keys(rf)) if (!keep.has(k)) delete rf[k];
  for (const k of Object.keys(dc)) if (!keep.has(k)) delete dc[k];
  return { ...base, rf, dc };
}

export function TrapPanel() {
  const s = useStore();
  const app = s.app;
  const sol = s.solution;
  const roles = useMemo(() => (app ? electrodeRoles(app) : {}), [app]);
  const p = s.trapInput;
  const setP = (trapInput: TrapParams) => useStore.setState({ trapInput });
  const [withDepth, setWithDepth] = useState(false);
  const [running, setRunning] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // set up for a new solution (keeping the current drive), not on every visit to the tab
  useEffect(() => {
    const st = useStore.getState();
    if (sol && app && (st.trapInput === null || st.trapInputFor !== sol.version)) {
      useStore.setState({ trapInput: withDefaults(st.trapInput ?? app.project.trap_params, sol, roles), trapInputFor: sol.version });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sol?.version]);

  // analyze whenever the inputs change (debounced); results are milliseconds without the depth estimate
  const seq = useRef(0);
  useEffect(() => {
    if (!p || !sol || sol.stale) return;
    const id = ++seq.current;
    const t = window.setTimeout(async () => {
      setRunning(true);
      try {
        const r = await api.trap({ ...p, depth: withDepth });
        if (id === seq.current) {
          useStore.setState({ trap: r });
          setErr(null);
        }
      } catch (e) {
        if (id === seq.current) setErr((e as Error).message);
      } finally {
        if (id === seq.current) setRunning(false);
      }
    }, 250);
    return () => window.clearTimeout(t);
  }, [p, withDepth, sol]);

  if (!app?.model) return <div className="panel hint pad">Load a STEP file first.</div>;
  if (!sol || !p) return <div className="panel hint pad">Solve the boundary-value problem first (Solve tab).</div>;

  const electrodes = sol.electrodes;
  const setV = (kind: 'rf' | 'dc', name: string, v: number) => setP({ ...p, [kind]: { ...p[kind], [name]: v } });
  const rfElectrodes = electrodes.filter((e) => !e.floating && roles[e.name] === 'rf');
  const allRf = rfElectrodes.length > 0 && rfElectrodes.every((e) => p.rf[e.name] === p.rf[rfElectrodes[0].name]) ? p.rf[rfElectrodes[0].name] : null;

  return (
    <div className="panel">
      {sol.stale && <div className="warn" style={{ margin: 8 }}>The electrode grouping changed since the last solve; solve again.</div>}
      <Section title="Ion and drive">
        <div className="row">
          <select
            value={IONS.find(([, m]) => m === p.ion.mass_amu)?.[0] ?? ''}
            onChange={(e) => {
              const ion = IONS.find(([n]) => n === e.target.value);
              if (ion) setP({ ...p, ion: { ...p.ion, mass_amu: ion[1] } });
            }}
          >
            <option value="">custom</option>
            {IONS.map(([n]) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <NumberField label="m" suffix="u" value={p.ion.mass_amu} onChange={(v) => setP({ ...p, ion: { ...p.ion, mass_amu: v } })} />
          <NumberField label="Q" suffix="e" value={p.ion.charge_e} onChange={(v) => setP({ ...p, ion: { ...p.ion, charge_e: v } })} />
        </div>
        <div className="row">
          <NumberField label="Ω/2π" suffix="MHz" value={Number((p.rf_freq / 1e6).toPrecision(8))} onChange={(v) => setP({ ...p, rf_freq: v * 1e6 })} />
          <NumberField
            label="V_rf"
            suffix="V"
            value={allRf ?? 0}
            title="Set the RF amplitude (zero to peak) of every electrode with the RF role"
            onChange={(v) => setP({ ...p, rf: { ...p.rf, ...Object.fromEntries(rfElectrodes.map((e) => [e.name, v])) } })}
          />
        </div>
        <table className="bodies compact">
          <thead>
            <tr>
              <th>Electrode</th>
              <th title="RF amplitude, zero to peak (negative = opposite phase)">RF (V)</th>
              <th title="Static voltage">DC (V)</th>
            </tr>
          </thead>
          <tbody>
            {electrodes.map((e) => {
              const role = (roles[e.name] ?? 'unassigned') as keyof typeof ROLE_COLORS;
              return (
                <tr key={e.name}>
                  <td>
                    <span className="electrode">
                      <span className="role-dot" style={{ background: ROLE_COLORS[role] }} title={ROLE_NAMES[role]} />
                      {e.name}
                    </span>
                  </td>
                  {e.floating ? (
                    <td colSpan={2} className="hint">
                      floating
                    </td>
                  ) : (
                    <>
                      <td>
                        <NumberField value={p.rf[e.name] ?? 0} onChange={(v) => setV('rf', e.name, v)} />
                      </td>
                      <td>
                        <NumberField value={p.dc[e.name] ?? 0} onChange={(v) => setV('dc', e.name, v)} />
                      </td>
                    </>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
        <div className="row">
          <button onClick={() => setP(withDefaults({ ...p, rf: {}, dc: {} }, sol, roles, allRf ?? 100))} title="RF amplitude on RF-role electrodes, 0 V everywhere else">
            Reset from roles
          </button>
          <label className="check" title="Ray-based estimate from the surface charges (about a second)">
            <input type="checkbox" checked={withDepth} onChange={(e) => setWithDepth(e.target.checked)} />
            trap depth
          </label>
          <span className="spacer" />
          {running && <span className="busy small">analyzing…</span>}
        </div>
        {err && <div className="error">{err}</div>}
      </Section>
      {s.trap && <SliceSection r={s.trap} p={p} />}
      {s.trap?.analysis.confined && <SweepSection p={p} electrodes={electrodes.filter((e) => !e.floating).map((e) => e.name)} />}
      {s.trap?.analysis.confined && <ChainSection p={p} />}
    </div>
  );
}

/** Figures and tables of the trap analysis, for the results area. */
export function TrapResults() {
  const s = useStore();
  const r = s.trap;
  const p = s.trapInput;
  if (!s.solution) return <div className="results-empty">Solve the boundary-value problem first (Solve tab).</div>;
  if (!r || !p) return <div className="results-empty">Set the ion and drive in the sidebar; the analysis runs as you type.</div>;
  return (
    <div className="cards">
      <Summary r={r} />
      <Modes r={r} />
      <Stability r={r} />
      {r.analysis.confined && <AxisProfiles r={r} />}
      {s.slice && <SliceCard sl={s.slice} r={r} />}
      <SweepCards />
      <ChainCards />
    </div>
  );
}

function Summary({ r }: { r: TrapResult }) {
  const s = useStore();
  const a = r.analysis;
  const um = (x: number[]) => vec(x, 'um', 2);
  const offset = Math.hypot(...a.equilibrium.map((x, i) => x - a.rf_null[i]));
  const erf = Math.hypot(...a.rf_field_at_eq);
  return (
    <Card
      title="Single ion"
      right={
        <label className="check" title="Ion, RF null and principal axes in the 3D view">
          <input type="checkbox" checked={s.view.showModes} onChange={(e) => s.setView({ showModes: e.target.checked })} />
          show in 3D
        </label>
      }
    >
        <div className={a.confined ? 'ok-banner' : 'error'}>{a.confined ? 'Ion confined in all three directions' : 'Ion not confined'}</div>
        {a.notes.map((n, i) => (
          <div key={i} className="warn">
            {n}
          </div>
        ))}
        <KV k="Equilibrium" v={um(a.equilibrium)} />
        <KV k="RF null" v={um(a.rf_null)} title={a.rf_null_line ? 'The RF field vanishes along a line through this point' : undefined} />
        {a.rf_null_line && <KV k="RF null line along" v={`(${a.rf_null_line.map((x) => x.toFixed(3)).join(', ')})`} />}
        <KV k="Offset from null" v={`${(offset * 1e6).toFixed(3)} µm`} />
        <KV k="RF field at ion" v={`${sig(erf, 3)} V/m`} title="Amplitude of the RF field at the equilibrium: drives excess micromotion" />
        {r.depth_ev !== null && r.depth && (
          <>
            <KV k="Trap depth ≈" v={`${fmtEv(r.depth_ev)} (${sig(r.depth_ev * EV_K, 3)} K)`} title={`Lowest barrier along ${r.depth.n_rays} straight rays from the ion; the true saddle can be lower`} />
            <KV k="Escape toward" v={`(${r.depth.escape_direction.map((x) => x.toFixed(2)).join(', ')}) via ${um(r.depth.barrier_point)}`} />
          </>
        )}
        <p className="hint">
          3D view: magenta ion, white RF null, colored principal axes. Expansions valid within {len(r.r_valid, 'um', 0)} of {um(r.center)}.
        </p>
    </Card>
  );
}

function Modes({ r }: { r: TrapResult }) {
  const a = r.analysis;
  return (
    <Card title="Secular modes" table>
        <table className="bodies compact">
          <thead>
            <tr>
              <th />
              <th title="Pseudopotential (adiabatic) frequency">f pseudo</th>
              <th title="From the Floquet multipliers of the linearized Mathieu equations">f Floquet</th>
              <th>axis</th>
            </tr>
          </thead>
          <tbody>
            {a.modes.map((m, i) => (
              <tr key={i}>
                <td>
                  <span className="swatch" style={{ background: MODE_COLORS[i] }} />
                </td>
                <td className="num">{fmtFreq(m.freq_pseudo)}</td>
                <td className="num">{fmtFreq(m.freq_floquet)}</td>
                <td className="num mono">({m.axis.map((x) => x.toFixed(3)).join(', ')})</td>
              </tr>
            ))}
          </tbody>
        </table>
        <h4>Anharmonicity along the modes</h4>
        <table className="bodies compact">
          <thead>
            <tr>
              <th />
              <th title="Amplitude at which the cubic term of U equals the harmonic one: c2/|c3|">L₃</th>
              <th title="Amplitude at which the quartic term equals the harmonic one: √(c2/|c4|); + stiffening, − softening">L₄</th>
            </tr>
          </thead>
          <tbody>
            {a.modes.map((m, i) => (
              <tr key={i}>
                <td>
                  <span className="swatch" style={{ background: MODE_COLORS[i] }} />
                </td>
                <td className="num">{m.c3 !== 0 && m.c2 > 0 ? fmtLen(Math.abs(m.c2 / m.c3)) : '—'}</td>
                <td className="num">{m.c4 !== 0 && m.c2 > 0 ? `${m.c4 > 0 ? '+' : '−'}${fmtLen(Math.sqrt(Math.abs(m.c2 / m.c4)))}` : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <h4>Mathieu parameters (RF quadrupole axes)</h4>
        <table className="bodies compact">
          <thead>
            <tr>
              <th>axis</th>
              <th title="a = 4Q·uᵀH_dc·u/(mΩ²)">a</th>
              <th title="q = −2Q·λ_rf/(mΩ²), λ_rf: eigenvalue of the RF potential Hessian">q</th>
            </tr>
          </thead>
          <tbody>
            {a.mathieu.map((m, i) => (
              <tr key={i}>
                <td className="num mono">({m.axis.map((x) => x.toFixed(3)).join(', ')})</td>
                <td className="num">{sig(m.a, 3)}</td>
                <td className="num">{sig(m.q, 3)}</td>
              </tr>
            ))}
          </tbody>
        </table>
    </Card>
  );
}

/** For each vector of `from`, the index of a vector of `to` (one to one, by largest total |cos|). */
function matchAxes(from: Vec3[], to: Vec3[]): number[] {
  const n = Math.min(from.length, to.length);
  const dot = (a: Vec3, b: Vec3) => Math.abs(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]);
  let best = from.map((_, i) => i % Math.max(1, to.length));
  let bestScore = -1;
  const perm = (used: number[], rest: number[]) => {
    if (used.length === n) {
      const sc = used.reduce((acc, j, i) => acc + dot(from[i], to[j]), 0);
      if (sc > bestScore) [bestScore, best] = [sc, [...used]];
      return;
    }
    rest.forEach((j) => perm([...used, j], rest.filter((x) => x !== j)));
  };
  if (n > 0 && n <= 6) perm([], to.map((_, j) => j));
  return best;
}

/** Mathieu a, q of each RF quadrupole axis on the stability diagram, with the sweep path. */
function Stability({ r }: { r: TrapResult }) {
  const sweep = useStore((st) => st.sweep);
  const rfFreq = useStore((st) => st.trapInput?.rf_freq);
  const [view, setView] = useState<'zoom' | 'full'>('zoom');
  const a = r.analysis;
  // each RF axis takes the color of the secular mode it is closest to
  const match = useMemo(() => matchAxes(a.mathieu.map((m) => m.axis), a.modes.map((m) => m.axis)), [a]);
  const label = (v: Vec3) => `RF axis (${v.map((x) => x.toFixed(2)).join(', ')})`;
  const points: OperatingPoint[] = useMemo(
    () => a.mathieu.map((m, i) => ({ label: label(m.axis), color: MODE_COLORS[match[i]] ?? MODE_COLORS[i], q: m.q, a: m.a })),
    [a, match],
  );
  const paths: Path[] = useMemo(() => {
    if (!sweep || !sweep.rows.some((row) => row.q?.length)) return [];
    return a.mathieu.map((m, i) => ({
      label: `${label(m.axis)} sweep`,
      color: MODE_COLORS[match[i]] ?? MODE_COLORS[i],
      points: sweep.rows.map((row) => [Math.abs(row.q?.[i] ?? NaN), row.a?.[i] ?? NaN] as [number, number]),
    }));
  }, [sweep, a, match]);
  return (
    <Card
      title="Stability diagram"
      right={
        <Seg
          value={view}
          onChange={setView}
          options={[
            { value: 'zoom', label: 'operating point', title: 'Zoom to the operating points' },
            { value: 'full', label: 'full', title: 'First stability region with iso-β lines' },
          ]}
        />
      }
    >
      <StabilityDiagram name="Mathieu stability diagram" points={points} paths={paths} rfFreq={rfFreq} view={view} height={280} />
      <p className="hint">
        Each RF quadrupole axis obeys u″ + (a − 2q cos 2τ)u = 0 (τ = Ωt/2): stable in the shaded band between a₀(q) and b₁(q).
        {paths.length > 0 && ` Lines: the ${sweep?.label} sweep.`} Hover for β and the Mathieu secular frequency βΩ/2.
      </p>
    </Card>
  );
}

/**
 * U along each principal axis through the ion, from the surface charges (the full
 * potential, not the expansion), against the harmonic approximation.
 */
function AxisProfiles({ r }: { r: TrapResult }) {
  const s = useStore();
  const auto = Number((0.5 * r.r_valid * 1e6).toPrecision(1)) * 1e-6;
  const [range, setRange] = useState<number | null>(null);
  const L = range ?? auto;
  const [data, setData] = useState<{ t: number[]; u: number[][] } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const p = useStore.getState().trapInput;
    if (!p) return;
    let live = true;
    const a = r.analysis;
    const N = 121;
    const ts = Array.from({ length: N }, (_, i) => -L + (2 * L * i) / (N - 1));
    const pts: Vec3[] = a.modes.flatMap((m) => ts.map((t) => [0, 1, 2].map((k) => a.equilibrium[k] + t * m.axis[k]) as Vec3));
    Promise.all([api.evaluate(pts, p.rf), api.evaluate(pts, p.dc)])
      .then(([rf, dc]) => {
        if (!live) return;
        const m = p.ion.mass_amu * AMU;
        const W = 2 * Math.PI * p.rf_freq;
        const Q = p.ion.charge_e;
        // eV: Q² e |∇φ_rf|² / (4 m Ω²) + Q φ_dc
        const U = pts.map((_, k) => {
          const g = rf.grad[k];
          return (Q * Q * E_CHARGE * (g[0] ** 2 + g[1] ** 2 + g[2] ** 2)) / (4 * m * W * W) + Q * dc.phi[k];
        });
        const u = a.modes.map((_, i) => {
          const row = U.slice(i * N, (i + 1) * N);
          const u0 = row[(N - 1) / 2];
          return row.map((x) => x - u0);
        });
        setData({ t: ts, u });
        setErr(null);
      })
      .catch((e) => live && setErr((e as Error).message));
    return () => {
      live = false;
    };
  }, [r, L]);

  const p = s.trapInput;
  const m = (p?.ion.mass_amu ?? 1) * AMU;
  return (
    <Card
      wide
      title="Pseudopotential along the mode axes"
      right={<LengthField label="±" value={L} units="um" onChange={(v) => setRange(v > 0 ? v : null)} title="Displacement range from the ion" />}
    >
      {err && <div className="error">{err}</div>}
      {data && (
        <div className="multiples">
          {r.analysis.modes.map((mode, i) => {
            const w = 2 * Math.PI * (mode.freq_pseudo ?? 0);
            const series: Series[] = [
              { name: 'full', color: MODE_COLORS[i], points: data.t.map((t, k) => [t * 1e6, data.u[i][k] * 1e3]) },
              { name: 'harmonic', color: '#8b93a1', dash: true, points: data.t.map((t) => [t * 1e6, ((0.5 * m * w * w * t * t) / E_CHARGE) * 1e3]) },
            ];
            return (
              <LinePlot
                key={i}
                title={`mode ${i + 1}: ${fmtFreq(mode.freq_pseudo)} along (${mode.axis.map((x) => x.toFixed(2)).join(', ')})`}
                series={series}
                xLabel="displacement (µm)"
                yLabel="U − U_ion (meV)"
                height={200}
                formatY={(y) => sig(y, 3)}
              />
            );
          })}
        </div>
      )}
      <p className="hint">
        Total pseudopotential plus static potential along each principal axis through the ion, from the surface charges; dashed: ½mω²x² with the
        pseudopotential secular frequency. Where they part, the trap is anharmonic.
      </p>
    </Card>
  );
}

function SliceCard({ sl, r }: { sl: SliceState; r: TrapResult }) {
  const s = useStore();
  const d = sl.data;
  const rgba = useMemo(() => sliceRgba(d.values, d.n, sl.ref, sl.clip), [d, sl.ref, sl.clip]);
  const AX = 'xyz';
  const rel = (q: Vec3) => [(q[d.u_axis] - d.center[d.u_axis]) * 1e6, (q[d.v_axis] - d.center[d.v_axis]) * 1e6];
  const [eu, ev] = rel(r.analysis.equilibrium);
  const [nu, nv] = rel(r.analysis.rf_null);
  const ions = (s.chain?.positions ?? []).map((q) => rel(q));
  return (
    <Card
      className="tall"
      title={`Pseudopotential in the plane ⟂ ${AX[d.axis]} through the ion`}
      right={
        <label className="check">
          <input type="checkbox" checked={s.view.showSlice} onChange={(e) => s.setView({ showSlice: e.target.checked })} />
          show in 3D
        </label>
      }
    >
      <HeatMap
        n={d.n}
        half={d.half * 1e6}
        rgba={rgba}
        uLabel={`${AX[d.u_axis]} − ${AX[d.u_axis]}_ion (µm)`}
        vLabel={`${AX[d.v_axis]} − ${AX[d.v_axis]}_ion (µm)`}
        markers={[
          ...ions.map(([u, v], k) => ({ u, v, color: ION_COLOR, label: `ion ${k + 1}`, r: 3 })),
          { u: nu, v: nv, color: '#ffffff', label: 'RF null', ring: true, r: 6 },
          { u: eu, v: ev, color: ION_COLOR, label: 'single-ion equilibrium', r: 5 },
        ]}
        readout={(i, j) => {
          const v = d.values[j * d.n + i];
          return v === null ? 'inside an electrode' : `U − U_ion = ${fmtEv(v - sl.ref)}`;
        }}
        name={`Pseudopotential, plane ⟂ ${AX[d.axis]} through the ion`}
        values={d.values.map((v) => (v === null ? null : v - sl.ref))}
        valueLabel="U − U_ion (eV)"
        colorBar={{
          stops: [0, 0.25, 0.5, 0.75, 1].map(viridisCss),
          lo: '0',
          hi: fmtEv(sl.clip),
          label: `U − U_ion, contours every ${fmtEv(sl.clip / 10)}`,
        }}
      />
      <div className="legend">
        <div className="ramp" style={{ background: `linear-gradient(90deg, ${[0, 0.25, 0.5, 0.75, 1].map(viridisCss).join(', ')})` }} />
        <div className="row spread mono small">
          <span>0</span>
          <span>U − U_ion, contours every {fmtEv(sl.clip / 10)}</span>
          <span>{fmtEv(sl.clip)}</span>
        </div>
      </div>
    </Card>
  );
}

function SliceSection({ r, p }: { r: TrapResult; p: TrapParams }) {
  const s = useStore();
  const [axis, setAxis] = useState<'0' | '1' | '2'>('0');
  const [half, setHalf] = useState(r.r_valid);
  const [n, setN] = useState(121);
  const [clip, setClip] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const key = JSON.stringify(p);
  const [sliceKey, setSliceKey] = useState('');
  const sl = s.slice;

  const compute = async () => {
    setBusy(true);
    setErr(null);
    try {
      const data = await api.slice({ ...p, depth: false }, Number(axis), r.analysis.equilibrium, half, n);
      useStore.setState({ slice: { data, ...sliceScale(data, r.energy_eq_ev, clip ?? r.depth_ev) } });
      s.setView({ showSlice: true });
      setSliceKey(key);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Pseudopotential slice"
      right={
        sl && (
          <label className="check">
            <input type="checkbox" checked={s.view.showSlice} onChange={(e) => s.setView({ showSlice: e.target.checked })} />
            show
          </label>
        )
      }
    >
      <p className="hint">
        U = Q²|∇φ_rf|²/(4mΩ²) + Qφ_dc on a plane through the ion, from the surface charges directly (valid everywhere outside the electrodes).
      </p>
      <div className="row">
        <span className="hint">normal</span>
        <Seg
          value={axis}
          onChange={setAxis}
          options={[
            { value: '0', label: 'x' },
            { value: '1', label: 'y' },
            { value: '2', label: 'z' },
          ]}
        />
        <NumberField label="n" value={n} onChange={(v) => setN(Math.max(11, Math.min(301, Math.round(v))))} title="Grid points per side" />
      </div>
      <div className="row">
        <LengthField label="half-width" value={half} units={s.units} onChange={setHalf} />
        <NumberField
          label="max"
          suffix="eV"
          value={clip ?? Number((sl?.clip ?? r.depth_ev ?? 0).toPrecision(3))}
          onChange={(v) => {
            setClip(v > 0 ? v : null);
            if (sl && v > 0) useStore.setState({ slice: { ...sl, clip: v } });
          }}
          title="Top of the color scale (U − U_ion); contours every tenth"
        />
      </div>
      <div className="row">
        <button className="primary" disabled={busy} onClick={compute}>
          {busy ? 'Computing…' : 'Compute slice'}
        </button>
        {sl && sliceKey !== key && <span className="hint">inputs changed since this slice</span>}
      </div>
      {err && <div className="error">{err}</div>}
    </Section>
  );
}
