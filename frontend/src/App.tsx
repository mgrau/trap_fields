import { useEffect, useRef } from 'react';
import { ElectrodesPanel } from './panels/ElectrodesPanel';
import { GeometryPanel } from './panels/GeometryPanel';
import { MeshPanel } from './panels/MeshPanel';
import { SolvePanel, SolveResults } from './panels/SolvePanel';
import { StudyPanel, StudyResults } from './panels/StudyPanel';
import { TrapPanel, TrapResults } from './panels/TrapPanel';
import { RESULT_TABS, useStore, type Layout, type SectionAxis, type Tab } from './store';
import { len, num, unitLabel, vec } from './units';
import type { ViewDir } from './viewer/Viewer';
import { trapRegion, ViewerCanvas, viewerCommands } from './viewer/ViewerCanvas';
import { LengthField, Seg } from './widgets';

const TABS: { id: Tab; label: string }[] = [
  { id: 'geometry', label: 'Geometry' },
  { id: 'electrodes', label: 'Electrodes' },
  { id: 'mesh', label: 'Mesh' },
  { id: 'solve', label: 'Solve' },
  { id: 'trap', label: 'Trap' },
  { id: 'study', label: 'Study' },
];

const VIEWS: { d: ViewDir; label: string }[] = [
  { d: 'iso', label: 'Iso' },
  { d: '+x', label: '+X' },
  { d: '-x', label: '−X' },
  { d: '+y', label: '+Y' },
  { d: '-y', label: '−Y' },
  { d: '+z', label: '+Z' },
  { d: '-z', label: '−Z' },
];

function Toolbar() {
  const s = useStore();
  const v = s.view;
  const bb = s.app?.bbox;
  const sec = v.section;
  const axisIdx = sec.axis ? { x: 0, y: 1, z: 2 }[sec.axis] : 0;
  const lo = bb ? bb[axisIdx] : -1;
  const hi = bb ? bb[axisIdx + 3] : 1;
  return (
    <div className="toolbar">
      <div className="brand">trapfields</div>
      <div className="group" title="View from direction">
        {VIEWS.map((x) => (
          <button key={x.d} onClick={() => viewerCommands.view(x.d)} title={`View from ${x.label}`}>
            {x.label}
          </button>
        ))}
        <button onClick={() => viewerCommands.fit()} title="Fit (F)">
          Fit
        </button>
      </div>
      <Seg
        value={v.projection}
        onChange={(projection) => s.setView({ projection })}
        options={[
          { value: 'persp', label: 'Persp', title: 'Perspective projection' },
          { value: 'ortho', label: 'Ortho', title: 'Orthographic projection' },
        ]}
      />
      <Seg
        value={v.rotate}
        onChange={(rotate) => s.setView({ rotate })}
        options={[
          { value: 'orbit', label: 'Orbit', title: 'Turntable rotation (keeps the up axis)' },
          { value: 'free', label: 'Free', title: 'Unconstrained trackball rotation' },
        ]}
      />
      <div className="group">
        <button className={v.edges ? 'on' : ''} onClick={() => s.setView({ edges: !v.edges })} title="Show CAD edges">
          Edges
        </button>
        <button className={v.xray ? 'on' : ''} onClick={() => s.setView({ xray: !v.xray })} title="Semi-transparent bodies">
          X-ray
        </button>
        <button className={v.axes ? 'on' : ''} onClick={() => s.setView({ axes: !v.axes })} title="Origin axes (X red, Y green, Z blue)">
          Axes
        </button>
      </div>
      <div className="group" title="Section plane">
        <select
          value={sec.axis ?? ''}
          onChange={(e) => {
            const axis = (e.target.value || null) as SectionAxis;
            const k = axis ? { x: 0, y: 1, z: 2 }[axis] : 0;
            const offset = bb ? Math.min(Math.max(0, bb[k]), bb[k + 3]) : 0;
            s.setView({ section: { ...sec, axis, offset } });
          }}
        >
          <option value="">No section</option>
          <option value="x">Section ⟂ X</option>
          <option value="y">Section ⟂ Y</option>
          <option value="z">Section ⟂ Z</option>
        </select>
        {sec.axis && (
          <>
            <input
              type="range"
              min={lo}
              max={hi}
              step={(hi - lo) / 1000}
              value={sec.offset}
              onChange={(e) => s.setView({ section: { ...sec, offset: Number(e.target.value) } })}
            />
            <span className="mono small w6">
              {num(sec.offset, s.units, s.units === 'mm' ? 3 : 0)} {unitLabel(s.units)}
            </span>
            <button onClick={() => s.setView({ section: { ...sec, flip: !sec.flip } })} title="Flip which side is kept">
              ⇄
            </button>
          </>
        )}
      </div>
      <button className={s.tool === 'measure' ? 'on' : ''} onClick={() => s.set({ tool: s.tool === 'measure' ? 'select' : 'measure', pending: null })} title="Measure distance (M)">
        Measure
      </button>
      <div className="group" title="Show only the electrodes in a box around the trap center, and frame it">
        <button
          className={v.region ? 'on' : ''}
          disabled={!s.tess}
          onClick={() => {
            const on = !v.region;
            s.setView({ region: on });
            if (on) s.set({ focusRequest: s.focusRequest + 1 });
            else window.setTimeout(() => viewerCommands.fit(), 30);
          }}
        >
          Trap region
        </button>
        {v.region && (
          <>
            <LengthField label="±" value={trapRegion(s).half} units={s.units} onChange={(h) => s.setView({ regionHalf: h > 0 ? h : null })} title="Half-size of the box" />
            <button onClick={() => s.set({ focusRequest: s.focusRequest + 1 })} title="Frame the trap region">
              Frame
            </button>
          </>
        )}
      </div>
      <div className="spacer" />
      {RESULT_TABS.includes(s.tab) && (
        <Seg<Layout>
          value={s.layout[s.tab]}
          onChange={(l) => s.set({ layout: { ...s.layout, [s.tab]: l } })}
          options={[
            { value: '3d', label: '3D', title: '3D view only' },
            { value: 'split', label: 'Split', title: '3D view and results side by side (drag the divider)' },
            { value: 'results', label: 'Results', title: 'Figures and tables only' },
          ]}
        />
      )}
      <Seg
        value={s.units}
        onChange={(units) => s.set({ units })}
        options={[
          { value: 'mm', label: 'mm' },
          { value: 'um', label: 'µm' },
        ]}
      />
    </div>
  );
}

function StatusBar() {
  const s = useStore();
  const h = s.hover;
  const app = s.app;
  const body = h && app ? app.bodies[h.body] : null;
  const job = app?.job;
  return (
    <div className="statusbar">
      <span className="mono">{h ? `${vec(h.point, s.units)}` : '—'}</span>
      {h && (
        <span>
          {body?.name ?? ''} {h.face >= 0 ? `· face ${h.face}` : ''} · {h.snap}
        </span>
      )}
      {s.tool === 'measure' && <span className="accent">{s.pending ? 'Measure: click the second point (Esc to cancel)' : 'Measure: click the first point'}</span>}
      {s.pending && h && <span className="mono accent">d = {len(Math.hypot(h.point[0] - s.pending[0], h.point[1] - s.pending[1], h.point[2] - s.pending[2]), s.units)}</span>}
      <div className="spacer" />
      {s.busy && <span className="busy">{s.busy}…</span>}
      {job?.state === 'running' && (
        <>
          <span className="busy">
            {job.kind === 'load' ? 'Loading geometry' : job.kind === 'mesh' ? 'Meshing' : job.kind === 'solve' ? `Solving (${job.stage ?? '…'})` : job.kind === 'study' ? `Study: ${job.stage ?? '…'}` : job.kind}… {(job.elapsed ?? 0).toFixed(0)} s
          </span>
          {job.kind !== 'load' && (
            <button className="danger small" disabled={job.stopping} onClick={() => s.stop()}>
              {job.stopping ? 'Stopping…' : 'Stop'}
            </button>
          )}
        </>
      )}
      {app?.tess && <span>{app.tess.n_triangles.toLocaleString()} display triangles</span>}
      {s.backendLabel && <span title="Where the computation runs">{s.backendLabel}</span>}
    </div>
  );
}

/** Drag handle between the 3D view and the results. */
function Splitter() {
  const ref = useRef<HTMLDivElement>(null);
  const onDown = (e: React.PointerEvent) => {
    const ws = ref.current?.parentElement;
    if (!ws) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const r = ws.getBoundingClientRect();
      useStore.setState({ splitFrac: Math.min(0.8, Math.max(0.15, (ev.clientX - r.left) / r.width)) });
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return <div ref={ref} className="splitter" onPointerDown={onDown} title="Drag to resize" />;
}

export function App() {
  const s = useStore();
  const layout: Layout = RESULT_TABS.includes(s.tab) ? s.layout[s.tab] : '3d';

  useEffect(() => {
    s.init();
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest('input, select, textarea')) return;
      const st = useStore.getState();
      if (e.key === 'Escape') st.set({ pending: null, tool: 'select', selected: null });
      else if (e.key === 'm' || e.key === 'M') st.set({ tool: st.tool === 'measure' ? 'select' : 'measure', pending: null });
      else if (e.key === 'f' || e.key === 'F') viewerCommands.fit();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="app">
      <Toolbar />
      <div className="main">
        <aside className="sidebar">
          <nav className="tabs">
            {TABS.map((t) => (
              <button key={t.id} className={s.tab === t.id ? 'on' : ''} onClick={() => s.set({ tab: t.id })}>
                {t.label}
              </button>
            ))}
          </nav>
          {s.error && (
            <div className="error banner" onClick={() => s.set({ error: null })} title="Dismiss">
              {s.error}
            </div>
          )}
          <div className="tab-body">
            {s.tab === 'geometry' && <GeometryPanel />}
            {s.tab === 'electrodes' && <ElectrodesPanel />}
            {s.tab === 'mesh' && <MeshPanel />}
            {s.tab === 'solve' && <SolvePanel />}
            {s.tab === 'trap' && <TrapPanel />}
            {s.tab === 'study' && <StudyPanel />}
          </div>
        </aside>
        <div className={`workspace layout-${layout}`} style={{ ['--split' as string]: `${(s.splitFrac * 100).toFixed(1)}%` }}>
          <ViewerCanvas />
          {layout === 'split' && <Splitter />}
          {layout !== '3d' && (
            <div className="results">
              {s.tab === 'solve' && <SolveResults />}
              {s.tab === 'trap' && <TrapResults />}
              {s.tab === 'study' && <StudyResults />}
            </div>
          )}
        </div>
      </div>
      <StatusBar />
    </div>
  );
}
