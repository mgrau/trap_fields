import { useEffect, useRef, useState } from 'react';
import { api, type AppStateDto, type FeatureFace, type TransformOp, type Vec3 } from '../api';
import { clearFeaturePick, useFeatures } from '../features';
import { download } from '../results';
import { useStore } from '../store';
import { len, num, unitLabel, vec, type Units } from '../units';
import { KV, LengthField, NextStep, NumberField, Section } from '../widgets';

const AXES: { name: string; v: Vec3 }[] = [
  { name: 'X', v: [1, 0, 0] },
  { name: 'Y', v: [0, 1, 0] },
  { name: 'Z', v: [0, 0, 1] },
];

function describe(op: TransformOp, u: Units): string {
  if (op.op === 'translate') return `Translate ${vec(op.delta, u)}`;
  const ax = AXES.find((a) => a.v.every((x, i) => x === op.axis[i]))?.name ?? `(${op.axis.map((x) => x.toFixed(3)).join(', ')})`;
  const c = op.center && op.center.some((x) => x !== 0) ? ` about ${vec(op.center, u)}` : '';
  return `Rotate ${op.angle_deg.toFixed(3).replace(/\.?0+$/, '')}° about ${ax}${c}`;
}

/** Rotation taking unit vector a onto b, as an axis-angle op. */
function alignOp(a: Vec3, b: Vec3, center: Vec3): TransformOp | null {
  const n = Math.hypot(...a);
  if (n === 0) return null;
  const u = a.map((x) => x / n) as Vec3;
  const cross: Vec3 = [u[1] * b[2] - u[2] * b[1], u[2] * b[0] - u[0] * b[2], u[0] * b[1] - u[1] * b[0]];
  const s = Math.hypot(...cross);
  const c = u[0] * b[0] + u[1] * b[1] + u[2] * b[2];
  if (s < 1e-12) {
    if (c > 0) return null; // already aligned
    const perp: Vec3 = Math.abs(u[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
    return { op: 'rotate', axis: perp, angle_deg: 180, center };
  }
  return { op: 'rotate', axis: cross.map((x) => x / s) as Vec3, angle_deg: (Math.atan2(s, c) * 180) / Math.PI, center };
}

/** "2 × Cylinder, Cone" */
function kinds(faces: FeatureFace[]): string {
  const n = new Map<string, number>();
  for (const f of faces) n.set(f.kind || 'face', (n.get(f.kind || 'face') ?? 0) + 1);
  return [...n].map(([k, c]) => (c > 1 ? `${c} × ${k}` : k)).join(', ');
}

/**
 * Remove holes, threads and similar features from the bodies (OpenCASCADE defeaturing):
 * pick their faces in the viewer, then remove them. Stored in the project and re-applied
 * on every load; the STEP file is not modified.
 */
function RemoveFeatures({ app, running }: { app: AppStateDto; running: boolean }) {
  const s = useStore();
  const pick = useFeatures();
  const picking = s.tool === 'features';
  const removed = app.project.removed_features ?? [];
  const area = pick.faces.reduce((a, f) => a + f.area, 0);
  // picking belongs to this panel: leaving it ends the tool
  useEffect(
    () => () => {
      if (useStore.getState().tool === 'features') useStore.getState().set({ tool: 'select' });
      clearFeaturePick();
    },
    [],
  );
  return (
    <Section
      title="Remove features"
      info="Screw holes, threads and chamfers far from the ion barely change the field but cost many mesh panels (modeled threads may not mesh at all). Pick a feature's faces and remove them: OpenCASCADE deletes the faces and extends the neighboring ones to close the gap. Removals are stored in the project and re-applied on every load; the STEP file is not modified."
      right={
        <button
          className={picking ? 'on' : ''}
          disabled={running}
          title="Click faces of a hole or thread in the 3D view"
          onClick={() => {
            s.set({ tool: picking ? 'select' : 'features', pending: null });
            if (picking) clearFeaturePick();
          }}
        >
          {picking ? 'Picking…' : 'Pick faces'}
        </button>
      }
    >
      {picking ? (
        <p className="hint">
          Click the wall of a hole or thread. {pick.grow ? 'The connected faces of the feature are added. ' : ''}Shift-click adds or removes single faces;
          Esc ends picking.
        </p>
      ) : (
        <p className="hint">Press Pick faces, then click a hole or thread in the 3D view.</p>
      )}
      <label className="check">
        <input type="checkbox" checked={pick.grow} onChange={(e) => useFeatures.setState({ grow: e.target.checked })} />
        Grow to the whole feature (walls, chamfers, countersink, bottom)
      </label>
      {pick.faces.length > 0 && pick.body !== null && (
        <div className="feature-pick">
          <div>
            <b>{app.bodies[pick.body]?.name}</b>: {pick.faces.length} face{pick.faces.length > 1 ? 's' : ''} ({kinds(pick.faces)}),{' '}
            {(area * 1e6).toPrecision(3)} mm²
          </div>
          <div className="row">
            <button
              className="primary"
              disabled={running}
              onClick={() => s.run('Removing features', () => api.removeFeatures(pick.faces.map((f) => f.tag)))}
            >
              Remove {pick.faces.length > 1 ? `${pick.faces.length} faces` : 'face'}
            </button>
            <button onClick={clearFeaturePick}>Clear</button>
          </div>
        </div>
      )}
      <h4>Removed</h4>
      {removed.length === 0 ? (
        <div className="hint">None — the bodies as in the STEP file.</div>
      ) : (
        <ol className="ops">
          {removed.map((r, i) => (
            <li key={i}>
              {r.body}: {r.faces.length} face{r.faces.length > 1 ? 's' : ''}, {(r.faces.reduce((a, f) => a + f.area, 0) * 1e6).toPrecision(3)} mm²
            </li>
          ))}
        </ol>
      )}
      <div className="row">
        <button disabled={running || !removed.length} title="Restore the last removed feature" onClick={() => s.run('Undo', api.undoFeatures)}>
          Undo
        </button>
        <button disabled={running || !removed.length} title="Restore every removed feature" onClick={() => s.run('Restoring features', api.resetFeatures)}>
          Restore all
        </button>
      </div>
    </Section>
  );
}

export function GeometryPanel() {
  const s = useStore();
  const app = s.app;
  const fileInput = useRef<HTMLInputElement>(null);
  const bundleInput = useRef<HTMLInputElement>(null);
  const [angle, setAngle] = useState(90);
  const [axis, setAxis] = useState(2);
  const [delta, setDelta] = useState<Vec3>([0, 0, 0]);
  const [aboutPicked, setAboutPicked] = useState(false);
  const u = s.units;
  const running = app?.job.state === 'running' || !!s.busy;
  const tr = (op: TransformOp) => s.run('Transforming', () => api.transform(op));
  const center: Vec3 = aboutPicked && s.picked ? s.picked.point : [0, 0, 0];
  const bb = app?.bbox;
  const last = s.measurements[s.measurements.length - 1];

  return (
    <div className="panel">
      <Section title="Project" info="The STEP file is never modified: orientation, labels and every result are stored next to it in the project folder (trapfields.json, mesh/, solve/, results/). Download project packs all of it into one .tar file.">
        <div className="row">
          <select
            value={app?.project.step_file ?? ''}
            onChange={(e) => e.target.value && s.run(`Loading ${e.target.value}`, () => api.open(e.target.value))}
            disabled={running}
          >
            <option value="">— choose a file in the project folder —</option>
            {app?.files.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
          <button
            disabled={running || !app?.project.step_file}
            title="Reload from disk"
            onClick={() => s.run('Reloading', () => api.open(app!.project.step_file!))}
          >
            ⟳
          </button>
        </div>
        <div className="row">
          <button disabled={running} onClick={() => fileInput.current?.click()}>
            Upload STEP…
          </button>
          <span className="hint">or drop a file on the viewport</span>
          <input
            ref={fileInput}
            type="file"
            accept=".step,.stp,.STEP,.STP"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) s.uploadStep(f);
              e.target.value = '';
            }}
          />
        </div>
        <div className="hint mono">{app?.project_dir}</div>
        <div className="row">
          <button
            disabled={running || !app?.project.step_file}
            title="The project file, its STEP file, mesh, solution and saved results, as one .tar file"
            onClick={() =>
              s.run('Packing the project', async () => {
                const tar = await api.exportProject();
                const stem = (app?.project.step_file ?? 'project').replace(/\.(step|stp)$/i, '');
                download(new Blob([tar], { type: 'application/x-tar' }), `${stem}.trapfields.tar`);
              })
            }
          >
            Download project
          </button>
          <button disabled={running} title="Replace this project with a downloaded one" onClick={() => bundleInput.current?.click()}>
            Open project…
          </button>
          <input
            ref={bundleInput}
            type="file"
            accept=".tar"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = '';
              if (!f) return;
              if (app?.project.step_file && !window.confirm(`Replace the current project (${app.project.step_file}, its mesh, solution and results) with ${f.name}?`)) return;
              s.run(`Opening ${f.name}`, () => api.importProject(f));
            }}
          />
        </div>
      </Section>

      {app?.model && (
        <Section title="Model">
          <KV k="File" v={app.model.file_name} />
          {app.model.products.length > 0 && <KV k="Design" v={app.model.products.join(', ')} />}
          {app.model.originating_system && <KV k="Exported by" v={app.model.originating_system} />}
          <KV k="File units" v={app.model.length_unit} />
          <KV k="Bodies / faces" v={`${app.bodies.length} / ${app.model.n_faces}`} />
          <KV k="Load + tessellate" v={`${(app.model.load_seconds + (app.tess?.seconds ?? 0)).toFixed(1)} s`} />
          {bb && (
            <>
              <KV k="Extent X" v={`${len(bb[3] - bb[0], u)}   [${num(bb[0], u)}, ${num(bb[3], u)}]`} />
              <KV k="Extent Y" v={`${len(bb[4] - bb[1], u)}   [${num(bb[1], u)}, ${num(bb[4], u)}]`} />
              <KV k="Extent Z" v={`${len(bb[5] - bb[2], u)}   [${num(bb[2], u)}, ${num(bb[5], u)}]`} />
            </>
          )}
          {[...app.model.warnings, ...(app.notes ?? [])].map((w) => (
            <div key={w} className="warn">
              {w}
            </div>
          ))}
          {!!app.tess?.failed_faces.length && (
            <div className="warn">
              {app.tess.failed_faces.length} face(s) could not be tessellated (e.g. modeled threads) and are not shown. Remove features below
              can often take them out (pick a neighboring face with Grow on); else simplify them in CAD.
            </div>
          )}
        </Section>
      )}

      {app?.model && <RemoveFeatures app={app} running={running} />}

      {app?.model && (
        <Section title="Orientation" info="Rigid transforms, stored in the project (the STEP file is not modified). The simulation frame is the transformed frame; the trap center should end up at the origin.">
          <div className="grid3">
            {AXES.map((a) => (
              <div key={a.name} className="rot">
                <span>{a.name}</span>
                <button disabled={running} title={`Rotate −90° about ${a.name}`} onClick={() => tr({ op: 'rotate', axis: a.v, angle_deg: -90, center })}>
                  −90°
                </button>
                <button disabled={running} title={`Rotate +90° about ${a.name}`} onClick={() => tr({ op: 'rotate', axis: a.v, angle_deg: 90, center })}>
                  +90°
                </button>
              </div>
            ))}
          </div>
          <div className="row">
            <NumberField label="Angle" value={angle} suffix="°" onChange={setAngle} />
            <select value={axis} onChange={(e) => setAxis(Number(e.target.value))}>
              {AXES.map((a, i) => (
                <option key={a.name} value={i}>
                  about {a.name}
                </option>
              ))}
            </select>
            <button disabled={running} onClick={() => tr({ op: 'rotate', axis: AXES[axis].v, angle_deg: angle, center })}>
              Rotate
            </button>
          </div>
          <label className="check">
            <input type="checkbox" checked={aboutPicked} onChange={(e) => setAboutPicked(e.target.checked)} disabled={!s.picked} />
            Rotate about picked point {s.picked ? vec(s.picked.point, u, 3) : '(click the model first)'} instead of origin
          </label>

          <h4>Translate</h4>
          <div className="row">
            {(['dx', 'dy', 'dz'] as const).map((l, i) => (
              <LengthField
                key={l}
                label={l}
                value={delta[i]}
                units={u}
                onChange={(m) => setDelta(delta.map((x, j) => (j === i ? m : x)) as Vec3)}
              />
            ))}
            <button disabled={running || delta.every((x) => x === 0)} onClick={() => tr({ op: 'translate', delta })}>
              Move
            </button>
          </div>
          <div className="row">
            <button
              disabled={running || !bb}
              title="Translate so the bounding-box center is at the origin"
              onClick={() => bb && tr({ op: 'translate', delta: [0, 1, 2].map((k) => -(bb[k] + bb[k + 3]) / 2) as Vec3 })}
            >
              Center bbox at origin
            </button>
            <button
              disabled={running || !s.picked}
              title="Translate so the last picked point becomes the origin"
              onClick={() => s.picked && tr({ op: 'translate', delta: s.picked.point.map((x) => -x) as Vec3 })}
            >
              Picked point → origin
            </button>
          </div>

          <h4>Align</h4>
          <div className="row">
            <span className="hint">Last measurement A→B along</span>
            {AXES.map((a) => (
              <button
                key={a.name}
                disabled={running || !last}
                title={`Rotate (about A) so that A→B points along +${a.name}`}
                onClick={() => {
                  const op = last && alignOp([last.b[0] - last.a[0], last.b[1] - last.a[1], last.b[2] - last.a[2]], a.v, last.a);
                  if (op) tr(op);
                }}
              >
                +{a.name}
              </button>
            ))}
          </div>

          <h4>Applied transforms</h4>
          {app.project.transforms.length === 0 ? (
            <div className="hint">None — STEP frame.</div>
          ) : (
            <ol className="ops">
              {app.project.transforms.map((op, i) => (
                <li key={i}>{describe(op, u)}</li>
              ))}
            </ol>
          )}
          <div className="row">
            <button disabled={running || !app.project.transforms.length} onClick={() => s.run('Undo', api.undo)}>
              Undo
            </button>
            <button disabled={running || !app.project.transforms.length} onClick={() => s.run('Reset', api.resetTransforms)}>
              Reset
            </button>
          </div>
        </Section>
      )}

      {app?.model && (
        <Section
          title="Measure"
          info="The cursor snaps to CAD vertices (green) and edges (amber), else picks the surface. Distances are exact for vertices and edges and within the tessellation tolerance on curved surfaces."
          right={
            <button className={s.tool === 'measure' ? 'on' : ''} onClick={() => s.set({ tool: s.tool === 'measure' ? 'select' : 'measure', pending: null })}>
              {s.tool === 'measure' ? 'Measuring…' : 'Measure (M)'}
            </button>
          }
        >
          <p className="hint">Click two points on the model.</p>
          {s.picked && <KV k="Picked" v={`${vec(s.picked.point, u)} · ${s.picked.snap}`} />}
          {s.measurements.map((m, i) => {
            const d: Vec3 = [m.b[0] - m.a[0], m.b[1] - m.a[1], m.b[2] - m.a[2]];
            return (
              <div key={m.id} className="measure">
                <div className="row">
                  <b>#{i + 1}</b>
                  <span className="mono">{len(Math.hypot(...d), u)}</span>
                  <button className="x" title="Remove" onClick={() => s.set({ measurements: s.measurements.filter((x) => x.id !== m.id) })}>
                    ×
                  </button>
                </div>
                <div className="mono small">
                  Δ = ({num(d[0], u)}, {num(d[1], u)}, {num(d[2], u)}) {unitLabel(u)}
                </div>
                <div className="mono small">
                  A {vec(m.a, u)} <br />B {vec(m.b, u)}
                </div>
              </div>
            );
          })}
          {s.measurements.length > 0 && <button onClick={() => s.set({ measurements: [], pending: null })}>Clear measurements</button>}
        </Section>
      )}
      {app?.model && <NextStep tab="electrodes" label="electrode roles" />}
    </div>
  );
}
