// Trap tab: frequency sweeps and N-ion chains (controls for the sidebar, cards for the results area).
import { useState } from 'react';
import { api, type SweepVar, type TrapParams, type Vec3 } from '../api';
import { MODE_COLORS } from '../colors';
import { LinePlot, type Series } from '../plot/LinePlot';
import { sweepMeta } from '../results';
import { useStore } from '../store';
import { viewerCommands } from '../viewer/ViewerCanvas';
import { Card, KV, NumberField, Section } from '../widgets';
import { sig } from './SolvePanel';

const AXIS = ['x', 'y', 'z'];
const dominant = (v: Vec3) => [0, 1, 2].reduce((b, k) => (Math.abs(v[k]) > Math.abs(v[b]) ? k : b), 0);

export function SweepSection({ p, electrodes }: { p: TrapParams; electrodes: string[] }) {
  const firstRf = Object.values(p.rf).find((v) => v !== 0) ?? 100;
  const [varKey, setVarKey] = useState('rf_amplitude');
  const [range, setRange] = useState<Record<string, [number, number]>>({});
  const [n, setN] = useState(41);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const dcName = varKey.startsWith('dc:') ? varKey.slice(3) : null;
  const unit = varKey === 'rf_freq' ? 'MHz' : 'V';
  const scale = varKey === 'rf_freq' ? 1e6 : 1;
  const current = varKey === 'rf_amplitude' ? Math.abs(firstRf) : varKey === 'rf_freq' ? p.rf_freq / 1e6 : (p.dc[dcName ?? ''] ?? 0);
  const [lo, hi] = range[varKey] ?? (varKey === 'rf_amplitude' ? [current * 0.3, current * 2] : varKey === 'rf_freq' ? [current * 0.5, current * 2] : [0, Math.max(20, current * 2)]);
  const variable: SweepVar = varKey === 'rf_amplitude' ? { kind: 'rf_amplitude' } : varKey === 'rf_freq' ? { kind: 'rf_freq' } : { kind: 'dc', electrode: dcName! };

  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      const values = Array.from({ length: n }, (_, i) => (lo + ((hi - lo) * i) / Math.max(1, n - 1)) * scale);
      const rows = (await api.sweep({ ...p, depth: false }, variable, values)).rows;
      useStore.setState({ sweep: { rows, ...sweepMeta(variable) } });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section title="Frequency sweep" defaultOpen={false}>
      <p className="hint">Single-ion secular frequencies (pseudopotential) while one drive parameter varies; gaps mark unconfined points.</p>
      <div className="row">
        <select value={varKey} onChange={(e) => setVarKey(e.target.value)}>
          <option value="rf_amplitude">RF amplitude (all RF electrodes)</option>
          <option value="rf_freq">RF frequency</option>
          {electrodes.map((e) => (
            <option key={e} value={`dc:${e}`}>
              DC voltage: {e}
            </option>
          ))}
        </select>
      </div>
      <div className="grid3">
        <NumberField label="from" suffix={unit} value={Number(lo.toPrecision(4))} onChange={(v) => setRange({ ...range, [varKey]: [v, hi] })} />
        <NumberField label="to" suffix={unit} value={Number(hi.toPrecision(4))} onChange={(v) => setRange({ ...range, [varKey]: [lo, v] })} />
        <NumberField label="n" value={n} onChange={(v) => setN(Math.max(2, Math.min(400, Math.round(v))))} />
      </div>
      <div className="row">
        <button className="primary" disabled={busy} onClick={run}>
          {busy ? 'Sweeping…' : 'Sweep'}
        </button>
        <span className="hint">current: {sig(current, 4)} {unit}</span>
      </div>
      {err && <div className="error">{err}</div>}
    </Section>
  );
}

export function SweepCards() {
  const sw = useStore((st) => st.sweep);
  if (!sw) return null;
  const { rows, scale, unit, label } = sw;
  const n = rows.length;
  // group modes by the Cartesian axis they point along, so crossing frequencies keep their identity
  const freqSeries: Series[] = [0, 1, 2].map((ax) => ({
    name: `mode along ${AXIS[ax]}`,
    color: MODE_COLORS[ax],
    markers: n <= 25,
    points: rows.map((r) => {
      const k = r.axes ? r.axes.findIndex((a) => dominant(a) === ax) : -1;
      const f = k >= 0 && r.confined ? r.freq_pseudo?.[k] : null;
      return [r.value / scale, f ? f / 1e3 : NaN] as [number, number];
    }),
  }));
  const qSeries: Series[] = [
    {
      name: '|q| (largest)',
      color: MODE_COLORS[0],
      points: rows.map((r) => [r.value / scale, r.q ? Math.max(...r.q.map(Math.abs)) : NaN] as [number, number]),
    },
  ];

  return (
    <>
      <Card title={`Secular frequencies against ${label}`}>
        <LinePlot series={freqSeries} xLabel={`${label} (${unit})`} yLabel="frequency (kHz)" height={240} />
        <p className="hint">Pseudopotential frequencies, each mode colored by the axis it points along; gaps mark unconfined points.</p>
      </Card>
      <Card title={`Stability parameter against ${label}`}>
        <LinePlot series={qSeries} xLabel={`${label} (${unit})`} yLabel="|q|" height={240} refLines={[{ y: 0.4, label: 'pseudopotential limit ~0.4' }]} />
      </Card>
    </>
  );
}

export function ChainSection({ p }: { p: TrapParams }) {
  const s = useStore();
  const [n, setN] = useState(5);
  const res = s.chain;
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      const r = await api.chain({ ...p, depth: false }, n);
      useStore.setState({ chain: r, ions: r.positions });
    } catch (e) {
      setErr((e as Error).message);
      useStore.setState({ chain: null, ions: null });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Ion chain"
      defaultOpen={false}
      right={
        res && (
          <label className="check">
            <input type="checkbox" checked={s.view.showIons} onChange={(e) => s.setView({ showIons: e.target.checked })} />
            show
          </label>
        )
      }
    >
      <p className="hint">
        Equilibrium of N ions (trap + Coulomb repulsion, all 3N coordinates) and its normal modes from the full Hessian. Starts from a linear
        chain along the weakest axis; zigzag configurations are found when the linear chain is unstable.
      </p>
      <div className="row">
        <NumberField label="N ions" value={n} onChange={(v) => setN(Math.max(1, Math.min(200, Math.round(v))))} />
        <button className="primary" disabled={busy} onClick={run}>
          {busy ? 'Solving…' : 'Find equilibrium'}
        </button>
      </div>
      {err && <div className="error">{err}</div>}
      {res && res.n > 1 && (
        <div className="row">
          <button onClick={() => zoomToChain(res.positions)} title="Frame the ions in the 3D view">
            Zoom to the ions
          </button>
        </div>
      )}
    </Section>
  );
}

function zoomToChain(q: Vec3[]) {
  const c = [0, 1, 2].map((k) => q.reduce((a, x) => a + x[k], 0) / q.length) as Vec3;
  const r = Math.max(...q.map((x) => Math.hypot(x[0] - c[0], x[1] - c[1], x[2] - c[2])));
  viewerCommands.focus(c, Math.max(r * 1.4, 5e-6));
}

export function ChainCards() {
  const res = useStore((st) => st.chain);
  if (!res) return null;
  const ax = res?.principal_axes;
  const rel = (q: Vec3, k: number) => (ax ? ax[k][0] * q[0] + ax[k][1] * q[1] + ax[k][2] * q[2] : 0);
  const center = res ? [0, 1, 2].map((k) => res.positions.reduce((a, q) => a + q[k], 0) / res.n) as Vec3 : null;
  const posSeries: Series[] = res && center
    ? [1, 2].map((k) => ({
        name: `offset along radial axis ${k}`,
        color: MODE_COLORS[k],
        pointsOnly: true,
        points: res.positions.map((q) => {
          const d: Vec3 = [q[0] - center[0], q[1] - center[1], q[2] - center[2]];
          return [rel(d, 0) * 1e6, rel(d, k) * 1e6] as [number, number];
        }),
      }))
    : [];
  const modeAxis = (w: [number, number, number]) => [0, 1, 2].reduce((b, k) => (w[k] > w[b] ? k : b), 0);
  const specSeries: Series[] = res
    ? [0, 1, 2].map((k) => ({
        name: k === 0 ? 'along the chain' : `radial ${k}`,
        color: MODE_COLORS[k],
        pointsOnly: true,
        points: res.modes.map((m, i) => [i + 1, modeAxis(m.axis_weights) === k ? m.freq / 1e3 : NaN] as [number, number]),
      }))
    : [];

  return (
    <>
      <Card title={`Ion chain: ${res.n} ion${res.n === 1 ? '' : 's'}`}>
        <KV k="Configuration" v={res.n < 2 ? 'single ion' : res.transverse < 0.01 ? 'linear chain' : `non-linear (zigzag), transverse ${(res.transverse * 100).toFixed(0)} % of spacing`} />
        <KV k="Minimum spacing" v={res.n > 1 ? `${(res.min_spacing * 1e6).toFixed(3)} µm` : '—'} />
        <KV k="Length" v={res.n > 1 ? `${((rel(res.positions[res.n - 1], 0) - rel(res.positions[0], 0)) * 1e6).toFixed(2)} µm` : '—'} />
        <KV k="Stable minimum" v={res.stable ? 'yes' : 'no (a saddle: increase the radial confinement)'} />
        <LinePlot series={posSeries} xLabel="position along the chain (µm)" yLabel="offset (µm)" height={200} />
      </Card>
      <Card title="Normal modes of the chain">
        <LinePlot series={specSeries} xLabel="mode" yLabel="frequency (kHz)" height={220} formatX={(x) => String(Math.round(x))} />
        <div className="table-scroll">
          <table className="bodies compact">
            <thead>
              <tr>
                <th>#</th>
                <th>frequency</th>
                <th>along chain</th>
                <th>radial 1</th>
                <th>radial 2</th>
              </tr>
            </thead>
            <tbody>
              {res.modes.map((m, i) => (
                <tr key={i}>
                  <td className="num">{i + 1}</td>
                  <td className="num">{(m.freq / 1e3).toFixed(3)} kHz</td>
                  {m.axis_weights.map((w, k) => (
                    <td key={k} className="num">
                      {(w * 100).toFixed(1)} %
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </>
  );
}
