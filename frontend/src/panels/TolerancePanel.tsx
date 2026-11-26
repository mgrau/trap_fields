// Study → Tolerances: how the trap responds to displacing each electrode. The sidebar
// runs the sensitivity study (one solve per electrode, degree of freedom and direction);
// the results show the derivatives of the trap quantities for the Trap tab's drive and a
// tolerance budget for given displacement spreads.
import { useEffect, useMemo, useState } from 'react';
import { api, type ToleranceEval, type ToleranceParam, type ToleranceQuantity } from '../api';
import { useStore } from '../store';
import { Card, GoTo, JobButton, JobOutcome, KV, NumberField, Section, Seg } from '../widgets';
import { sig } from './SolvePanel';

type Group = ToleranceQuantity['group'];

const GROUPS: { value: Group; label: string; title: string }[] = [
  { value: 'position', label: 'Ion', title: 'Equilibrium position of the ion' },
  { value: 'null', label: 'RF null', title: 'Position of the RF null (on a null line: where it passes the ion)' },
  { value: 'field', label: 'RF field', title: 'RF field amplitude at the ion: excess micromotion' },
  { value: 'frequency', label: 'Frequencies', title: 'Secular frequencies (pseudopotential)' },
  { value: 'tilt', label: 'Axes', title: 'Tilt of the principal axes' },
];

/** Display scale and unit of a quantity. */
const Q_UNIT: Record<ToleranceQuantity['unit'], [number, string]> = { m: [1e6, 'µm'], 'V/m': [1, 'V/m'], Hz: [1, 'Hz'], rad: [1e3, 'mrad'] };
/** Per µm of translation, per mrad of rotation. */
const P_UNIT: Record<ToleranceParam['kind'], [number, string]> = { translate: [1e-6, 'µm'], rotate: [1e-3, 'mrad'] };

const fmt = (v: number | null | undefined, digits = 3) => (v === null || v === undefined || !Number.isFinite(v) ? '—' : sig(v, digits));

/** The drive the Trap tab uses (the saved one if the tab was not opened yet). */
function useDrive() {
  const s = useStore();
  return s.trapInput ?? s.app?.project.trap_params ?? null;
}

export function TolerancePanel() {
  const s = useStore();
  const app = s.app!;
  const centers = app.electrode_centers ?? {};
  const names = Object.keys(centers);
  const [off, setOff] = useState<string[]>([]);
  const [tUm, setTUm] = useState(5);
  const [rMrad, setRMrad] = useState(1);
  const [central, setCentral] = useState(true);
  const chosen = names.filter((n) => !off.includes(n));
  const solves = 1 + chosen.length * 6 * (central ? 2 : 1);
  const t = app.tolerance;
  const busy = !!s.busy || app.job.state === 'running';
  const solveSeconds = app.solve?.seconds ?? null;

  if (!app.solve) {
    return (
      <p className="hint">
        Solve first: the study displaces the electrodes of the current solution. <GoTo tab="solve">Go to Solve →</GoTo>
      </p>
    );
  }
  return (
    <>
      <Section
        title="Sensitivity to displacements"
        info="Moves each chosen electrode by a small step along x, y and z and turns it by a small angle about x, y and z (through its center), solves again each time on the same, moved mesh, and keeps the expansions. The derivatives of the ion position, RF null, RF field at the ion, secular frequencies and principal axes then follow for any drive in milliseconds. Central differences (a step each way) are accurate to second order and also give the second derivatives. The solves use a tighter compression (ACA 1e-8) than the default, since the quantities are small differences between solves. Displacements set on the Electrodes tab stay in place: the study linearizes about the solved geometry."
      >
        <p className="hint">How much does the position of each electrode matter?</p>
        <div className="checks">
          {names.map((n) => (
            <label key={n} className="check">
              <input type="checkbox" checked={!off.includes(n)} onChange={(e) => setOff(e.target.checked ? off.filter((x) => x !== n) : [...off, n])} />
              {n}
            </label>
          ))}
        </div>
        <div className="grid2">
          <NumberField label="step" suffix="µm" value={tUm} onChange={(v) => v > 0 && v <= 1000 && setTUm(v)} title="Translation step (larger: less solver noise, more nonlinearity)" />
          <NumberField label="angle" suffix="mrad" value={rMrad} onChange={(v) => v > 0 && v <= 100 && setRMrad(v)} title="Rotation step" />
        </div>
        <label className="check">
          <input type="checkbox" checked={central} onChange={(e) => setCentral(e.target.checked)} />
          both directions (second derivatives; twice the solves)
        </label>
        <KV k="Solves" v={`${solves}${solveSeconds ? `, about ${fmtDuration(1.3 * solves * solveSeconds)}` : ''}`} title="One per electrode, degree of freedom and direction, plus the unperturbed geometry; the time from the last solve" />
        <div className="row">
          <JobButton
            kind="tolerance"
            label={`Run ${solves} solves`}
            disabled={busy || !chosen.length || app.solve.stale}
            title={app.solve.stale ? 'The electrodes changed since the solve: solve again first' : undefined}
            onClick={() => s.run('Starting tolerance study', () => api.tolerance({ electrodes: chosen, translate_step: tUm * 1e-6, rotate_step: rMrad * 1e-3, central }))}
          />
          {app.job.state === 'running' && app.job.kind === 'tolerance' && <span className="hint">{app.job.stage}</span>}
        </div>
        <JobOutcome kind="tolerance" />
        {app.solve.stale && <div className="warn">The electrodes or their displacements changed since the solve: solve again first.</div>}
        {t && (
          <KV
            k="Last study"
            v={`${t.electrodes.length} electrode${t.electrodes.length === 1 ? '' : 's'}, ±${sig(t.translate_step * 1e6, 3)} µm / ${sig(t.rotate_step * 1e3, 3)} mrad, ${fmtDuration(t.seconds)}${t.complete ? '' : ` (stopped: ${t.cases}/${t.planned})`}`}
          />
        )}
      </Section>
      <Section
        title="Tolerance budget"
        info="Every translation component and every rotation component of every studied electrode independently normally distributed with these standard deviations. The spread of each quantity is the quadrature sum of the contributions (to first order); the mean shift comes from the second derivatives (central differences only)."
      >
        <div className="grid2">
          <NumberField label="σ shift" suffix="µm" value={Number((s.tolSigma.translate * 1e6).toPrecision(6))} onChange={(v) => v >= 0 && s.set({ tolSigma: { ...s.tolSigma, translate: v * 1e-6 } })} />
          <NumberField label="σ angle" suffix="mrad" value={Number((s.tolSigma.rotate * 1e3).toPrecision(6))} onChange={(v) => v >= 0 && s.set({ tolSigma: { ...s.tolSigma, rotate: v * 1e-3 } })} />
        </div>
      </Section>
    </>
  );
}

function fmtDuration(s: number): string {
  if (s < 90) return `${s.toFixed(0)} s`;
  if (s < 5400) return `${(s / 60).toFixed(0)} min`;
  return `${(s / 3600).toFixed(1)} h`;
}

interface BudgetRow {
  q: ToleranceQuantity;
  index: number;
  sigma: number | null;
  shift: number | null;
  /** largest contributions: parameter index and share of the variance */
  top: { p: number; share: number }[];
}

/** σ_q² = Σ_p (∂q/∂p σ_p)², mean shift ½ Σ_p ∂²q/∂p² σ_p² (as tolerance.rs `budget`). */
function budget(e: ToleranceEval, sigma: { translate: number; rotate: number }): BudgetRow[] {
  const sp = e.params.map((p) => (p.kind === 'translate' ? sigma.translate : sigma.rotate));
  return e.quantities.map((q, qi) => {
    const parts: { p: number; v: number }[] = [];
    e.params.forEach((_, p) => {
      const d = e.d1[p]?.[qi];
      if (d !== null && d !== undefined) parts.push({ p, v: (d * sp[p]) ** 2 });
    });
    const variance = parts.reduce((a, x) => a + x.v, 0);
    const second = e.d2 ? e.params.map((_, p) => e.d2![p]?.[qi]).map((d, p) => (d === null || d === undefined ? null : d * sp[p] * sp[p])).filter((x): x is number => x !== null) : [];
    parts.sort((a, b) => b.v - a.v);
    return {
      q,
      index: qi,
      sigma: parts.length ? Math.sqrt(variance) : null,
      shift: second.length ? 0.5 * second.reduce((a, x) => a + x, 0) : null,
      top: variance > 0 ? parts.slice(0, 3).filter((x) => x.v > 0).map((x) => ({ p: x.p, share: x.v / variance })) : [],
    };
  });
}

export function ToleranceResults() {
  const s = useStore();
  const app = s.app!;
  const t = app.tolerance;
  const drive = useDrive();
  const [ev, setEv] = useState<ToleranceEval | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [group, setGroup] = useState<Group>('position');
  const running = app.job.state === 'running' && app.job.kind === 'tolerance';
  const key = t ? `${app.solve?.version}|${t.cases}|${t.seconds}|${JSON.stringify(drive)}` : '';

  useEffect(() => {
    setError(null);
    if (!t || !drive || running) return void (t ? null : setEv(null));
    let live = true;
    api
      .toleranceEval(drive, t.cases)
      .then((r) => live && setEv(r))
      .catch((e) => live && (setEv(null), setError((e as Error).message)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, running]);

  const rows = useMemo(() => (ev ? budget(ev, s.tolSigma) : []), [ev, s.tolSigma]);

  if (running) return <div className="results-empty">Tolerance study running: {app.job.stage}</div>;
  if (!t) return <div className="results-empty">Run a tolerance study (sidebar) to see how displacing each electrode changes the trap.</div>;
  if (!drive)
    return (
      <div className="results-empty">
        Set a drive on the Trap tab first: the sensitivities are for its voltages. <GoTo tab="trap">Go to Trap →</GoTo>
      </div>
    );
  if (error) return <div className="results-empty error">{error}</div>;
  if (!ev) return <div className="results-empty">Evaluating…</div>;

  const label = (p: ToleranceParam) => `${p.electrode} ${p.label}`;
  const inGroup = ev.quantities.map((q, i) => [q, i] as const).filter(([q]) => q.group === group);

  return (
    <div className="cards">
      <Card wide table title="Tolerance budget" right={<span className="hint">σ shift {sig(s.tolSigma.translate * 1e6, 3)} µm, σ angle {sig(s.tolSigma.rotate * 1e3, 3)} mrad per component</span>}>
        <div className="table-scroll">
          <table className="bodies compact">
            <thead>
              <tr>
                <th>quantity</th>
                <th title="Value for the unperturbed geometry">nominal</th>
                <th title="Standard deviation of its change, to first order">σ</th>
                <th title="Mean change from the second derivatives">mean shift</th>
                <th>largest contributions (share of σ²)</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const [k, u] = Q_UNIT[r.q.unit];
                return (
                  <tr key={r.q.key} className={r.q.undefined ? 'muted' : ''}>
                    <td>{r.q.name}</td>
                    <td className="num">{r.q.group === 'tilt' ? '' : `${fmt(r.q.nominal !== null ? r.q.nominal * k : null, 6)} ${u}`}</td>
                    <td className="num">{r.sigma !== null ? `${fmt(r.sigma * k)} ${u}` : '—'}</td>
                    <td className="num">{r.shift !== null ? `${fmt(r.shift * k, 2)} ${u}` : '—'}</td>
                    <td>
                      {r.top.map((x) => (
                        <span key={x.p} className="contrib" title={`${label(ev.params[x.p])}: ${(100 * x.share).toFixed(1)} % of the variance`}>
                          <span className="contrib-bar" style={{ width: `${Math.max(2, 40 * x.share)}px` }} />
                          {label(ev.params[x.p])} {(100 * x.share).toFixed(0)} %
                        </span>
                      ))}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Card>

      <Card wide table title="Sensitivities" right={<Seg value={group} onChange={setGroup} options={GROUPS.map((g) => ({ value: g.value, label: g.label, title: g.title }))} />}>
        <SensitivityTable ev={ev} columns={inGroup.map(([q, i]) => ({ q, i }))} />
        <p className="hint">
          Change per µm of translation and per mrad of rotation (about the electrode's center), {ev.central ? 'central differences' : 'forward differences'} with steps of{' '}
          {sig(ev.translate_step * 1e6, 3)} µm and {sig(ev.rotate_step * 1e3, 3)} mrad. Shading: size relative to the largest in the column.
        </p>
      </Card>

      {ev.notes.length > 0 && (
        <Card title="Notes">
          {ev.notes.map((n) => (
            <p key={n} className="hint">
              {n}
            </p>
          ))}
        </Card>
      )}
    </div>
  );
}

function SensitivityTable({ ev, columns }: { ev: ToleranceEval; columns: { q: ToleranceQuantity; i: number }[] }) {
  // per column, the largest size of a translation row and of a rotation row (in display units)
  const scaled = (p: number, i: number) => {
    const d = ev.d1[p]?.[i];
    const par = ev.params[p];
    const q = ev.quantities[i];
    return d === null || d === undefined ? null : d * Q_UNIT[q.unit][0] * P_UNIT[par.kind][0];
  };
  const maxOf = (i: number, kind: ToleranceParam['kind']) =>
    Math.max(1e-300, ...ev.params.map((p, j) => (p.kind === kind ? Math.abs(scaled(j, i) ?? 0) : 0)));
  const max = columns.map(({ i }) => ({ translate: maxOf(i, 'translate'), rotate: maxOf(i, 'rotate') }));
  return (
    <div className="table-scroll tall">
      <table className="bodies compact sens">
        <thead>
          <tr>
            <th>electrode</th>
            <th>moved</th>
            {columns.map(({ q }) => (
              <th key={q.key}>{q.name}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {ev.params.map((p, j) => (
            <tr key={`${p.electrode}|${p.dof}`} className={p.dof === 0 ? 'group-start' : ''}>
              <td>{p.dof === 0 || j === 0 || ev.params[j - 1].electrode !== p.electrode ? p.electrode : ''}</td>
              <td>{p.kind === 'translate' ? `Δ${p.label}` : p.label}</td>
              {columns.map(({ q, i }, c) => {
                const v = scaled(j, i);
                const shade = v === null ? 0 : Math.min(1, Math.abs(v) / max[c][p.kind]);
                return (
                  <td key={q.key} className="num" style={{ background: `color-mix(in srgb, var(--accent) ${(shade * 35).toFixed(0)}%, transparent)` }}>
                    {v === null ? '—' : `${fmt(v)} ${Q_UNIT[q.unit][1]}/${P_UNIT[p.kind][1]}`}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
