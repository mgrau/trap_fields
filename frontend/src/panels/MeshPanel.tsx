import { useEffect, useState } from 'react';
import { api, type MeshParams, type Vec3 } from '../api';
import { viridisCss } from '../colors';
import { estimatePanels, FINENESS, initialMesh, presetScale, scaledMesh } from '../meshPresets';
import { useStore } from '../store';
import { len, vec } from '../units';
import { JobButton, JobOutcome, KV, LengthField, Needs, NextStep, NumberField, Section, Seg } from '../widgets';

export function MeshPanel() {
  const s = useStore();
  const app = s.app;
  const u = s.units;
  const [p, setP] = useState<MeshParams | null>(null);

  // a different model invalidates the form
  const stepFile = app?.project.step_file;
  useEffect(() => setP(null), [stepFile]);
  // initialize from the last-used or suggested parameters
  useEffect(() => {
    if (!p && app) setP(initialMesh(app));
  }, [app, p]);

  if (!app?.model) return <Needs tab="geometry">Load a STEP file first.</Needs>;
  if (!p) return <div className="panel hint pad">No geometry around the origin to suggest mesh sizes.</div>;

  const set = (patch: Partial<MeshParams>) => setP({ ...p, ...patch });
  const stats = app.mesh?.stats;
  const range = s.meshSizeRange;
  const nearest = app.bodies.map((b) => b.closest?.distance ?? Infinity).reduce((a, b) => Math.min(a, b), Infinity);
  const base = app.mesh_defaults;
  const scale = presetScale(p, base);
  const estimate = estimatePanels(p, app.mesh);

  return (
    <div className="panel">
      <Section
        title="Surface mesh"
        info={`Surface mesh for the BEM solver, finest at the trap center (nearest electrode ${Number.isFinite(nearest) ? len(nearest, u) : '—'}) and coarser with distance. Finer meshes are more accurate and slower; a convergence study (Study tab) measures the error.`}
      >
        {base && (
          <div className="row">
            <span className="hint">Fineness</span>
            <Seg
              value={FINENESS.find((f) => f.scale === scale)?.key ?? ''}
              onChange={(k) => {
                const f = FINENESS.find((x) => x.key === k)!;
                setP(scaledMesh(base, f.scale, p.center));
              }}
              options={FINENESS.map((f) => ({ value: f.key, label: f.label, title: `${f.scale}× the suggested sizes` }))}
            />
            {scale === null && <span className="hint">custom</span>}
          </div>
        )}
        <KV
          k="Sizes"
          v={`${len(p.h_min, u, 3)} at the center → ${len(p.h_max, u, 2)} far away`}
          title="h_min within d_min of the center, growing linearly to h_max at d_max"
        />
        {estimate !== null && Math.abs(estimate - (app.mesh?.stats.n_triangles ?? 0)) > 1 && (
          <KV k="Expected" v={`≈ ${(Math.round(estimate / 1000) || 1).toLocaleString()}k panels (now ${((app.mesh?.stats.n_triangles ?? 0) / 1000).toFixed(0)}k)`} />
        )}
        {s.backendKind === 'browser' && (estimate ?? 0) > 45000 && (
          <p className="warn-text">About {Math.round((estimate ?? 0) / 1000)}k panels may be too many to solve in the browser (4 GB memory limit).</p>
        )}
        <div className="row">
          <JobButton kind="mesh" label="Generate mesh" disabled={!!s.busy} onClick={() => s.run('Starting mesher', () => api.mesh(p))} />
        </div>
        <JobOutcome kind="mesh" />
      </Section>

      <Section title="Advanced sizing" defaultOpen={scale === null}>
        <h4>Center (trap frame)</h4>
        <div className="row">
          {(['x', 'y', 'z'] as const).map((l, i) => (
            <LengthField key={l} label={l} value={p.center[i]} units={u} onChange={(m) => set({ center: p.center.map((x, j) => (j === i ? m : x)) as Vec3 })} />
          ))}
        </div>
        <div className="row">
          <button onClick={() => set({ center: [0, 0, 0] })}>Origin</button>
          <button disabled={!s.picked} onClick={() => s.picked && set({ center: s.picked.point })}>
            Picked point
          </button>
        </div>
        <div className="grid2">
          <LengthField label="h_min" value={p.h_min} units={u} onChange={(h_min) => set({ h_min })} title="Element size near the center" />
          <LengthField label="d_min" value={p.dist_min} units={u} onChange={(dist_min) => set({ dist_min })} title="Radius of the finest region" />
          <LengthField label="h_max" value={p.h_max} units={u} onChange={(h_max) => set({ h_max })} title="Element size far away" />
          <LengthField label="d_max" value={p.dist_max} units={u} onChange={(dist_max) => set({ dist_max })} title="Radius where h_max is reached" />
          <NumberField
            label="pts/circle"
            value={p.min_circle_points}
            onChange={(v) => set({ min_circle_points: Math.max(3, Math.round(v)) })}
            title="Minimum number of elements around a full circle"
          />
        </div>
        {p.dist_max > p.dist_min && (
          <p className="hint" title="Panel size grows by this much per unit distance between d_min and d_max; seen from the center, one panel spans about this angle">
            Grading: +{((p.h_max - p.h_min) / (p.dist_max - p.dist_min)).toFixed(3)} per unit distance, ≈{' '}
            {((Math.atan((p.h_max - p.h_min) / (p.dist_max - p.dist_min)) * 180) / Math.PI).toFixed(0)}° per panel seen from the center.
            {(p.h_max - p.h_min) / (p.dist_max - p.dist_min) > 0.25 && ' Steep: electrodes a few mm away (e.g. endcaps) are coarse; check with a convergence study.'}
          </p>
        )}
      </Section>

      {stats && app.mesh && (
        <Section
          title="Current mesh"
          right={
            <label className="check">
              <input type="checkbox" checked={s.view.showMesh} onChange={(e) => s.setView({ showMesh: e.target.checked })} />
              show
            </label>
          }
        >
          <KV k="Triangles" v={stats.n_triangles.toLocaleString()} />
          <KV k="Nodes" v={stats.n_nodes.toLocaleString()} />
          <KV k="Edge length" v={`${len(stats.edge_min, u, 2)} – ${len(stats.edge_max, u, 2)} (mean ${len(stats.edge_mean, u, 2)})`} />
          <KV k="Surface area" v={`${(stats.area_total * 1e6).toFixed(2)} mm²`} />
          <KV k="Time" v={`${stats.seconds.toFixed(1)} s`} />
          <KV k="Center" v={vec(app.mesh.params.center, u)} />
          {stats.defective_faces.length > 0 && (
            <div className="error">
              Folded or duplicate triangles remain on CAD faces {stats.defective_faces.join(', ')} after {stats.repair_rounds} repair rounds. The solver
              refuses this mesh; try a smaller h_max.
            </div>
          )}
          {stats.repair_rounds > 0 && stats.defective_faces.length === 0 && (
            <p className="hint">Repaired folded triangles by remeshing ({stats.repair_rounds} round{stats.repair_rounds > 1 ? 's' : ''}).</p>
          )}
          {stats.curvature_capped_faces > 0 && (
            <KV k="Curved faces" v={`${stats.curvature_capped_faces} capped at ≈ radius`} title="Panel size on curved faces is limited to ~1 radius of curvature so gmsh does not fold them" />
          )}
          {stats.failed_faces.length > 0 && (
            <div className="warn">
              {stats.failed_faces.length} CAD face(s) could not be meshed and are missing (tags {stats.failed_faces.slice(0, 8).join(', ')}
              {stats.failed_faces.length > 8 ? '…' : ''}). Acceptable only far from the trap.
            </div>
          )}
          <table className="bodies compact">
            <tbody>
              {app.bodies.map((b, i) => (
                <tr key={b.name}>
                  <td>{b.name}</td>
                  <td className="num">{stats.triangles_per_body[i]?.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="row">
            <span className="hint">Color</span>
            <Seg
              value={s.view.meshColor}
              onChange={(meshColor) => s.setView({ meshColor })}
              options={[
                { value: 'size', label: 'panel size' },
                { value: 'body', label: 'electrode' },
                ...(s.sigma ? [{ value: 'sigma' as const, label: 'charge' }] : []),
              ]}
            />
          </div>
          {s.view.meshColor === 'size' && range && (
            <div className="legend">
              <div className="ramp" style={{ background: `linear-gradient(90deg, ${[0, 0.25, 0.5, 0.75, 1].map(viridisCss).join(', ')})` }} />
              <div className="row spread mono small">
                <span>{len(range[0], u)}</span>
                <span>log scale</span>
                <span>{len(range[1], u)}</span>
              </div>
            </div>
          )}
          <p className="hint">Saved to mesh/surface_mesh.bin (+ .json) in the project folder.</p>
        </Section>
      )}
      {app.mesh && app.job.state !== 'running' && <NextStep tab="solve" label="solve" />}
    </div>
  );
}
