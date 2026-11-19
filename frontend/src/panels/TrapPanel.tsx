import { useEffect, useMemo, useRef, useState } from 'react';
import { api, type AppStateDto, type SolveSummary, type TrapParams, type TrapResult } from '../api';
import { MODE_COLORS, ROLE_COLORS, ROLE_NAMES, viridisCss } from '../colors';
import { useStore } from '../store';
import { len, vec } from '../units';
import { KV, LengthField, NumberField, Section, Seg } from '../widgets';
import { sig } from './SolvePanel';

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

const fmtFreq = (hz: number | null) => (hz === null ? '—' : hz >= 1e6 ? `${(hz / 1e6).toFixed(4)} MHz` : `${(hz / 1e3).toFixed(2)} kHz`);
const fmtLen = (m: number) => (m > 1 ? '> 1 m' : m >= 1e-3 ? `${(m * 1e3).toPrecision(3)} mm` : `${(m * 1e6).toPrecision(3)} µm`);
const fmtEv = (ev: number) => (Math.abs(ev) >= 0.1 ? `${ev.toFixed(3)} eV` : `${(ev * 1e3).toFixed(2)} meV`);

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
  const [p, setP] = useState<TrapParams | null>(null);
  const [withDepth, setWithDepth] = useState(false);
  const [running, setRunning] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // (re)initialize when a new solution arrives
  useEffect(() => {
    if (sol && app) setP(withDefaults(app.project.trap_params, sol, roles));
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
      {s.trap && <Results r={s.trap} p={p} />}
    </div>
  );
}

function Results({ r, p }: { r: TrapResult; p: TrapParams }) {
  const s = useStore();
  const a = r.analysis;
  const um = (x: number[]) => vec(x, 'um', 2);
  const offset = Math.hypot(...a.equilibrium.map((x, i) => x - a.rf_null[i]));
  const erf = Math.hypot(...a.rf_field_at_eq);
  return (
    <>
      <Section
        title="Trap"
        right={
          <label className="check">
            <input type="checkbox" checked={s.view.showModes} onChange={(e) => s.setView({ showModes: e.target.checked })} />
            show
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
        <h4>Secular modes</h4>
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
        <p className="hint">
          Magenta: ion; white: RF null; lines: principal axes. Expansions valid within {len(r.r_valid, 'um', 0)} of {um(r.center)}.
        </p>
      </Section>
      <SliceSection r={r} p={p} />
    </>
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
      const ref = r.energy_eq_ev;
      let c = clip ?? r.depth_ev ?? null;
      if (c === null || !(c > 0)) {
        const rel = data.values.filter((v): v is number => v !== null).map((v) => v - ref).sort((x, y) => x - y);
        c = rel[Math.floor(0.6 * (rel.length - 1))] || 1;
      }
      useStore.setState({ slice: { data, clip: c, ref } });
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
      {sl && (
        <div className="legend">
          <div className="ramp" style={{ background: `linear-gradient(90deg, ${[0, 0.25, 0.5, 0.75, 1].map(viridisCss).join(', ')})` }} />
          <div className="row spread mono small">
            <span>0</span>
            <span>U − U_ion, contours every {fmtEv(sl.clip / 10)}</span>
            <span>{fmtEv(sl.clip)}</span>
          </div>
        </div>
      )}
    </Section>
  );
}
