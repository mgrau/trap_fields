import { useEffect, useRef, useState } from 'react';
import { ElectrodesPanel } from './panels/ElectrodesPanel';
import { GeometryPanel } from './panels/GeometryPanel';
import { MeshPanel } from './panels/MeshPanel';
import { SolvePanel, SolveResults } from './panels/SolvePanel';
import { StudyPanel, StudyResults } from './panels/StudyPanel';
import { TrapPanel, TrapResults } from './panels/TrapPanel';
import { RESULT_TABS, useHover, useStore, type Layout, type Tab } from './store';
import { len, vec } from './units';
import { viewerCommands } from './viewer/commands';
import { ViewerCanvas } from './viewer/ViewerCanvas';
import { ErrorBoundary } from './ErrorBoundary';
import { activityFraction, busyFraction, jobFraction, useActivity } from './progress';
import { Seg } from './widgets';

const STEPS: { id: Tab; label: string }[] = [
  { id: 'geometry', label: 'Geometry' },
  { id: 'electrodes', label: 'Electrodes' },
  { id: 'mesh', label: 'Mesh' },
  { id: 'solve', label: 'Solve' },
  { id: 'trap', label: 'Trap' },
  { id: 'study', label: 'Study' },
];

type StepState = 'done' | 'warn' | 'todo';

/** Progress of each workflow step, for the step bar. */
function stepState(id: Tab, s: ReturnType<typeof useStore.getState>): { state: StepState; note: string } {
  const app = s.app;
  if (!app?.model) return { state: 'todo', note: id === 'geometry' ? 'Open a STEP file' : 'Load a geometry first' };
  switch (id) {
    case 'geometry':
      return { state: 'done', note: `${app.model.file_name}: ${app.bodies.length} bodies` };
    case 'electrodes': {
      const n = app.bodies.filter((b) => (b.label?.role ?? 'unassigned') === 'unassigned').length;
      return n ? { state: 'warn', note: `${n} bodies without a role` } : { state: 'done', note: 'Every body has a role' };
    }
    case 'mesh':
      if (!app.mesh) return { state: 'todo', note: 'No mesh yet' };
      return app.mesh.stats.defective_faces.length
        ? { state: 'warn', note: 'The mesh has defects' }
        : { state: 'done', note: `${app.mesh.stats.n_triangles.toLocaleString()} panels` };
    case 'solve':
      if (!app.solve) return { state: 'todo', note: 'Not solved yet' };
      return app.solve.stale ? { state: 'warn', note: 'Electrodes changed since the solve' } : { state: 'done', note: `${app.solve.n_panels.toLocaleString()} panels solved` };
    case 'trap':
      if (!s.trap) return { state: 'todo', note: app.solve ? 'Open to analyze the trap' : 'Solve first' };
      return s.trap.analysis.confined ? { state: 'done', note: 'Ion confined' } : { state: 'warn', note: 'Ion not confined' };
    case 'study':
      return app.study ? { state: 'done', note: `${app.study.levels.length}-level study` } : { state: 'todo', note: 'No study yet' };
  }
}

const MARK: Record<StepState, string> = { done: '✓', warn: '!', todo: '' };

function TopBar() {
  const s = useStore();
  const job = s.app?.job;
  return (
    <header className="topbar">
      <div className="brand">trapfields</div>
      <nav className="steps" aria-label="Workflow">
        {STEPS.map((t, i) => {
          const st = stepState(t.id, s);
          return (
            <button
              key={t.id}
              className={`step ${s.tab === t.id ? 'on' : ''} ${st.state}`}
              onClick={() => s.set({ tab: t.id })}
              title={st.note}
              aria-label={`${t.label}: ${st.note}`}
              aria-current={s.tab === t.id ? 'step' : undefined}
            >
              <span className="step-mark">{MARK[st.state] || i + 1}</span>
              {t.label}
            </button>
          );
        })}
      </nav>
      <div className="spacer" />
      {(s.busy || job?.state === 'running') && (
        <div className="job" role="status">
          <span className="busy">
            {job?.state === 'running' ? `${jobLabel(job.kind)}${job.stage ? `: ${job.stage}` : ''}` : s.busy}
            {job?.state === 'running' && <span className="job-time">{(job.elapsed ?? 0).toFixed(0)} s</span>}
          </span>
          {job?.state === 'running' && !FEATURE_JOBS.includes(job.kind ?? '') && (
            <button
              className="danger"
              disabled={job.stopping}
              onClick={() => s.stop()}
              title={
                s.backendKind === 'browser'
                  ? 'Stop at the next checkpoint; a step that cannot be interrupted (such as importing a big STEP file) is stopped by restarting the engine'
                  : 'Stop at the next checkpoint (a step that cannot be interrupted, such as importing a big STEP file, finishes first)'
              }
            >
              {job.stopping ? 'Stopping…' : 'Stop'}
            </button>
          )}
        </div>
      )}
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
          { value: 'mm', label: 'mm', title: 'Lengths in millimeters' },
          { value: 'um', label: 'µm', title: 'Lengths in micrometers' },
        ]}
      />
      <JobProgress />
    </header>
  );
}

// before a job reports its kind, the native server shows the command (open, upload, …)
const JOB_LABELS: Record<string, string> = {
  load: 'Loading',
  open: 'Loading',
  upload: 'Loading',
  startup: 'Loading',
  import_project: 'Loading',
  mesh: 'Meshing',
  solve: 'Solving',
  study: 'Study',
  remove_features: 'Removing features',
  features_undo: 'Restoring features',
  features_reset: 'Restoring features',
};
/** OpenCASCADE's defeaturing cannot be interrupted, and restarting the engine mid-way
 *  would replay the half-edited removal list: these jobs get no Stop button. */
const FEATURE_JOBS = ['remove_features', 'features_undo', 'features_reset'];
const jobLabel = (kind?: string) => (kind ? (JOB_LABELS[kind] ?? kind) : '');

/** Progress of whatever runs (a long job, else the longest quick command): fraction,
 *  label and whether anything runs, re-read every animation frame while it does. */
function useProgress(): { active: boolean; fraction: number; label: string } {
  const app = useStore((s) => s.app);
  const busy = useStore((s) => s.busy);
  const items = useActivity((s) => s.items);
  const running = app?.job.state === 'running';
  const active = running || items.length > 0 || !!busy;
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (!active) return;
    let raf = 0;
    const tick = () => {
      setNow(performance.now());
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active]);
  const job = jobFraction(app, now);
  const busyF = busyFraction(running ? null : busy, now);
  // a job's name and stage are in the top bar already
  if (running) return { active, fraction: job ?? 0, label: '' };
  const quick = activityFraction(items, now);
  if (quick) return { active, fraction: quick.fraction, label: quick.label };
  // "Connecting", "Starting mesher": short steps before a job reports its progress
  return { active, fraction: busyF, label: busy ?? '' };
}

const SHOW_AFTER_MS = 100;
const HOLD_FULL_MS = 450;

/**
 * Bar along the bottom edge of the top bar for anything that takes longer than
 * SHOW_AFTER_MS: it fills from 0 to 100 % over the whole job, then shows 100 % for a
 * moment before it fades, so short operations neither flash it nor vanish half-way.
 */
function JobProgress() {
  const { active, fraction, label } = useProgress();
  const [phase, setPhase] = useState<'idle' | 'waiting' | 'shown' | 'finishing'>('idle');
  const last = useRef({ fraction: 0, label: '' });
  useEffect(() => {
    if (active) {
      if (phase === 'idle' || phase === 'finishing') {
        setPhase('waiting');
      }
      return;
    }
    if (phase === 'shown') setPhase('finishing');
    else if (phase === 'waiting') setPhase('idle');
  }, [active, phase]);
  useEffect(() => {
    if (phase === 'waiting') {
      const t = window.setTimeout(() => setPhase('shown'), SHOW_AFTER_MS);
      return () => window.clearTimeout(t);
    }
    if (phase === 'finishing') {
      const t = window.setTimeout(() => setPhase('idle'), HOLD_FULL_MS);
      return () => window.clearTimeout(t);
    }
  }, [phase]);
  if (active) last.current = { fraction, label };
  if (phase !== 'shown' && phase !== 'finishing') return null;
  const f = phase === 'finishing' ? 1 : last.current.fraction;
  const pct = Math.round(100 * f);
  return (
    <div
      className={`job-progress ${phase === 'finishing' ? 'finishing' : ''}`}
      role="progressbar"
      aria-label={last.current.label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
    >
      <div className="job-progress-fill" style={{ width: `${(100 * f).toFixed(1)}%` }} />
      <span className="job-progress-pct">
        {last.current.label ? `${last.current.label} ` : ''}
        {pct} %
      </span>
    </div>
  );
}

function StatusBar() {
  const s = useStore();
  const h = useHover((x) => x.hover);
  const app = s.app;
  const body = h && app ? app.bodies[h.body] : null;
  return (
    <div className="statusbar">
      <span className="mono">{h ? `${vec(h.point, s.units)}` : '—'}</span>
      {h && (
        <span>
          {body?.name ?? ''} {h.face >= 0 ? `· face ${h.face}` : ''} · {h.snap}
        </span>
      )}
      {s.tool === 'measure' && <span className="accent">{s.pending ? 'Measure: click the second point (Esc to cancel)' : 'Measure: click the first point'}</span>}
      {s.tool === 'features' && <span className="accent">Remove features: click a hole or thread face; shift-click for single faces (Esc to end)</span>}
      {s.pending && h && <span className="mono accent">d = {len(Math.hypot(h.point[0] - s.pending[0], h.point[1] - s.pending[1], h.point[2] - s.pending[2]), s.units)}</span>}
      <div className="spacer" />
      {app?.tess && <span>{app.tess.n_triangles.toLocaleString()} display triangles</span>}
      {s.backendLabel && <span title="Where the computation runs">{s.backendLabel}</span>}
    </div>
  );
}

/** An in-page question (the store's `ask`), instead of the browser's confirm(). */
function AskDialog() {
  const ask = useStore((s) => s.ask);
  const set = useStore((s) => s.set);
  const yes = useRef<HTMLButtonElement>(null);
  useEffect(() => yes.current?.focus(), [ask]);
  if (!ask) return null;
  const close = () => set({ ask: null });
  return (
    <div className="ask-backdrop" onKeyDown={(e) => e.key === 'Escape' && close()} onClick={(e) => e.target === e.currentTarget && close()}>
      <div className="ask" role="alertdialog" aria-modal="true" aria-labelledby="ask-title" aria-describedby="ask-message">
        <h3 id="ask-title">{ask.title}</h3>
        <p id="ask-message">{ask.message}</p>
        <div className="ask-buttons">
          <button onClick={close}>Cancel</button>
          <button
            ref={yes}
            className="primary"
            onClick={() => {
              close();
              ask.onYes();
            }}
          >
            {ask.yes}
          </button>
        </div>
      </div>
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
  return <div ref={ref} className="splitter" onPointerDown={onDown} title="Drag to resize" role="separator" aria-orientation="vertical" />;
}

export function App() {
  const s = useStore();
  const layout: Layout = RESULT_TABS.includes(s.tab) ? s.layout[s.tab] : '3d';

  useEffect(() => {
    s.init();
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest('input, select, textarea')) return;
      const st = useStore.getState();
      if (e.key === 'Escape') st.set({ pending: null, tool: 'select', selected: null, selection: [] });
      else if (e.key === 'm' || e.key === 'M') st.set({ tool: st.tool === 'measure' ? 'select' : 'measure', pending: null });
      else if (e.key === 'f' || e.key === 'F') viewerCommands.fit();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="app">
      <TopBar />
      <div className="main">
        <aside className="sidebar">
          {s.error && (
            <div className="error banner" onClick={() => s.set({ error: null })} title="Dismiss">
              {s.error}
            </div>
          )}
          <div className="tab-body">
            <ErrorBoundary key={s.tab} label="This panel">
              {s.tab === 'geometry' && <GeometryPanel />}
              {s.tab === 'electrodes' && <ElectrodesPanel />}
              {s.tab === 'mesh' && <MeshPanel />}
              {s.tab === 'solve' && <SolvePanel />}
              {s.tab === 'trap' && <TrapPanel />}
              {s.tab === 'study' && <StudyPanel />}
            </ErrorBoundary>
          </div>
        </aside>
        <div className={`workspace layout-${layout}`} style={{ ['--split' as string]: `${(s.splitFrac * 100).toFixed(1)}%` }}>
          <ViewerCanvas />
          {layout === 'split' && <Splitter />}
          {layout !== '3d' && (
            <div className="results">
              <ErrorBoundary key={s.tab} label="The results">
                {s.tab === 'solve' && <SolveResults />}
                {s.tab === 'trap' && <TrapResults />}
                {s.tab === 'study' && <StudyResults />}
              </ErrorBoundary>
            </div>
          )}
        </div>
      </div>
      <StatusBar />
      <AskDialog />
    </div>
  );
}
