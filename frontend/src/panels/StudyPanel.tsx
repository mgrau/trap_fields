import { useEffect, useMemo, useState } from 'react';
import { api, type StudyLevel, type StudyQuantity } from '../api';
import { SERIES } from '../colors';
import { LinePlot } from '../plot/LinePlot';
import { initialMesh } from '../meshPresets';
import { useStore } from '../store';
import { Card, JobButton, JobOutcome, KV, Needs, Section } from '../widgets';
import { sig } from './SolvePanel';
import { Convergence } from './SolvePanel';

/** Quantities from levels (no extrapolation) while a study is still running. */
function liveQuantities(levels: StudyLevel[]): StudyQuantity[] {
  if (!levels.length) return [];
  return levels[0].values.map((v) => ({
    name: v.name,
    unit: v.unit,
    values: levels.map((l) => l.values.find((x) => x.name === v.name)?.value ?? null),
  }));
}

const fmtQ = (v: number | null | undefined, unit: string) => {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  if (unit === 'Hz') return `${(v / 1e3).toFixed(3)} kHz`;
  if (unit === 'm') return `${(v * 1e6).toFixed(3)} µm`;
  return sig(v, 6);
};

export function StudyPanel() {
  const s = useStore();
  const app = s.app;
  const [scalesText, setScalesText] = useState('2, 1.4, 1, 0.75');
  const [withTrap, setWithTrap] = useState(true);
  const running = app?.job.state === 'running' && app.job.kind === 'study';

  if (!app?.model) return <Needs tab="geometry">Load a STEP file first.</Needs>;
  const base = initialMesh(app);
  const solve = app.project.solve_params ?? { lmax: 10, center: null, r0: null, method: 'auto' as const };
  // the drive as set on the Trap tab (the saved one if the tab was not opened yet)
  const trap = s.trapInput ?? app.project.trap_params;
  const scales = scalesText
    .split(/[\s,]+/)
    .map(Number)
    .filter((x) => x > 0 && Number.isFinite(x));
  const busy = !!s.busy || app.job.state === 'running';

  return (
    <div className="panel">
      <Section
        title="Convergence study"
        info="Meshes and solves at several refinements (h_min and h_max of the current mesh sizes times each scale), tracking the potentials, fields and curvatures of every electrode at the center and the secular frequencies of the current drive. The three finest levels give an extrapolation f(s) ≈ f∞ + C·s^p. The current mesh and solution are not changed."
      >
        <p className="hint">How much do the results still change as the mesh gets finer?</p>
        <div className="row">
          <label className="field" title="Refinement factors, coarse to fine (smaller = finer)">
            <span className="field-label">scales</span>
            <input type="text" value={scalesText} onChange={(e) => setScalesText(e.target.value)} />
          </label>
        </div>
        <label className="check">
          <input type="checkbox" checked={withTrap && !!trap} disabled={!trap} onChange={(e) => setWithTrap(e.target.checked)} />
          secular frequencies for the Trap-tab drive {trap ? '' : '(set a drive on the Trap tab first)'}
        </label>
        <KV k="Base sizes" v={base ? `h_min ${(base.h_min * 1e6).toFixed(0)} µm, h_max ${(base.h_max * 1e3).toFixed(2)} mm` : '—'} />
        <KV k="Solver" v={`${solve.method ?? 'auto'}${solve.settings ? `, GMRES ${solve.settings.gmres_tol}, ACA ${solve.settings.aca_eps}` : ''}`} />
        <div className="row">
          <JobButton
            kind="study"
            label={`Run ${scales.length} levels`}
            disabled={busy || !base || scales.length < 1}
            onClick={() => s.run('Starting study', () => api.study({ base: base!, scales, solve, trap: withTrap ? trap : null }))}
          />
          {running && <span className="hint">{app.job.stage}</span>}
        </div>
        <JobOutcome kind="study" />
        {app.study && !running && (
          <KV
            k="Last study"
            v={`${app.study.levels.length} level${app.study.levels.length === 1 ? '' : 's'} in ${app.study.seconds.toFixed(0)} s${app.study.complete === false ? ' (stopped early)' : ''}`}
          />
        )}
      </Section>
    </div>
  );
}

/** Live progress, levels, convergence of a chosen quantity and the table of all of them. */
export function StudyResults() {
  const s = useStore();
  const app = s.app;
  const [pick, setPick] = useState<string | null>(null);
  const running = app?.job.state === 'running' && app.job.kind === 'study';
  const levels: StudyLevel[] = (running ? app?.progress?.study : app?.study?.levels) ?? [];
  const quantities = useMemo(() => (running || !app?.study ? liveQuantities(levels) : app.study.quantities), [running, app?.study, levels]);
  useEffect(() => {
    if (!pick && quantities.length) setPick(quantities.find((q) => q.name === 'mode 1 frequency')?.name ?? quantities[0].name);
  }, [quantities, pick]);
  if (!app) return null;
  if (!running && !levels.length) return <div className="results-empty">Run a study (sidebar) to see how the results converge with the mesh.</div>;
  const q = quantities.find((x) => x.name === pick) ?? null;
  const xs = levels.map((l) => l.n_panels);
  const tol = app.project.solve_params?.settings?.gmres_tol ?? 1e-9;

  return (
    <div className="cards">
      {running && <Convergence app={app} solution={null} tol={tol} live="study" />}

      {levels.length > 0 && (
        <Card title="Levels" table>
          <table className="bodies compact">
            <thead>
              <tr>
                <th>scale</th>
                <th>panels</th>
                <th>mesh</th>
                <th>solve</th>
                <th>GMRES</th>
              </tr>
            </thead>
            <tbody>
              {levels.map((l) => (
                <tr key={l.scale}>
                  <td className="num">{l.scale}</td>
                  <td className="num">{l.n_panels.toLocaleString()}</td>
                  <td className="num">{l.mesh_seconds.toFixed(1)} s</td>
                  <td className="num">{l.solve_seconds.toFixed(1)} s</td>
                  <td className="num">{l.iterations ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {q && (
        <Card
          title="Convergence of a quantity"
          right={
            <select value={pick ?? ''} onChange={(e) => setPick(e.target.value)}>
              {quantities.map((x) => (
                <option key={x.name} value={x.name}>
                  {x.name}
                </option>
              ))}
            </select>
          }
        >
          <LinePlot
            name={`Convergence of ${q.name}`}
            height={260}
            series={[
              {
                name: q.name,
                color: SERIES[0],
                markers: true,
                points: xs.map((x, i) => [x, q.values[i] ?? NaN] as [number, number]).filter((p) => Number.isFinite(p[1])),
              },
            ]}
            xLabel="panels"
            yLabel={q.unit}
            refLines={q.extrapolated !== undefined ? [{ y: q.extrapolated, label: `extrapolated ${fmtQ(q.extrapolated, q.unit)}` }] : []}
            formatX={(x) => (x >= 1000 ? `${(x / 1000).toFixed(1)}k` : String(x))}
            formatY={(y) => fmtQ(y, q.unit)}
          />
          {q.extrapolated !== undefined ? (
            <p className="hint">
              Finest {fmtQ(q.values[q.values.length - 1], q.unit)}, extrapolated {fmtQ(q.extrapolated, q.unit)} (order p ≈ {q.order?.toFixed(2)}),
              estimated error of the finest level {fmtQ(q.error_estimate, q.unit)}
              {q.values[q.values.length - 1] ? ` (${(((q.error_estimate ?? 0) / Math.abs(q.values[q.values.length - 1]!)) * 100).toFixed(2)} %)` : ''}.
            </p>
          ) : (
            <p className="hint">
              {running ? 'Extrapolation after the study finishes.' : 'No extrapolation: the three finest levels do not converge monotonically.'}
            </p>
          )}
        </Card>
      )}

      {!running && app.study && (
        <Card wide table title="All quantities" right={<span className="hint">click a row to plot it</span>}>
          <div className="table-scroll tall">
            <table className="bodies compact">
              <thead>
                <tr>
                  <th>quantity</th>
                  <th>finest</th>
                  <th>extrapolated</th>
                  <th>p</th>
                  <th>est. error</th>
                </tr>
              </thead>
              <tbody>
                {app.study.quantities.map((x) => {
                  const fin = x.values[x.values.length - 1];
                  const rel = x.error_estimate !== undefined && fin ? x.error_estimate / Math.abs(fin) : null;
                  return (
                    <tr key={x.name} onClick={() => setPick(x.name)} className={x.name === pick ? 'sel' : ''}>
                      <td>{x.name}</td>
                      <td className="num">{fmtQ(fin, x.unit)}</td>
                      <td className="num">{fmtQ(x.extrapolated, x.unit)}</td>
                      <td className="num">{x.order !== undefined && Number.isFinite(x.order) ? x.order.toFixed(2) : '—'}</td>
                      <td className="num">{rel !== null ? `${(rel * 100).toFixed(2)} %` : 'not monotone'}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  );
}
