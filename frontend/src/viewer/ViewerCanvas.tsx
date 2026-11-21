import { useEffect, useMemo, useRef, useState } from 'react';
import { MODE_COLORS, bodyColors, sliceImage } from '../colors';
import type { TrapResult, Vec3 } from '../api';
import { newMeasureId, selectBody, setHover, TRAP_TABS, useStore, type Tab } from '../store';
import { pickFeatureFace, useFeatures } from '../features';
import { len } from '../units';
import { setViewer as registerViewer, SIGMA_DECADES, trapRegion } from './commands';
import { Viewer, type CameraState } from './Viewer';
import { ViewerToolbar } from './ViewerToolbar';

/**
 * Looking down the trap axis (the RF null line, else the weakest mode), tilted by 25° so
 * the electrodes show depth.
 */
function axisView(t: TrapResult): { dir: Vec3; up: Vec3 } {
  const a = t.analysis;
  const ax = a.rf_null_line ?? [...a.modes].sort((p, q) => (p.freq_pseudo ?? 0) - (q.freq_pseudo ?? 0))[0]?.axis ?? [1, 0, 0];
  const up: Vec3 = Math.abs(ax[2]) > 0.9 ? [0, 1, 0] : [0, 0, 1];
  // a radial direction: up × axis
  const r: Vec3 = [up[1] * ax[2] - up[2] * ax[1], up[2] * ax[0] - up[0] * ax[2], up[0] * ax[1] - up[1] * ax[0]];
  const c = Math.cos((25 * Math.PI) / 180);
  const sn = Math.sin((25 * Math.PI) / 180);
  return { dir: [0, 1, 2].map((k) => c * ax[k] + sn * (0.8 * r[k] + 0.6 * up[k])) as Vec3, up };
}


/** Camera and region remembered separately for the model tabs and the trap tabs. */
type ViewContext = 'model' | 'trap';
const contextOf = (t: Tab): ViewContext => (TRAP_TABS.includes(t) ? 'trap' : 'model');

export function ViewerCanvas() {
  const host = useRef<HTMLDivElement>(null);
  const [viewer, setViewer] = useState<Viewer | null>(null);
  const [drag, setDrag] = useState(false);
  const s = useStore();

  useEffect(() => {
    const v = new Viewer(host.current!);
    registerViewer(v);
    setViewer(v);
    return () => {
      v.dispose();
      registerViewer(null);
    };
  }, []);

  // pointer callbacks read the latest store state
  useEffect(() => {
    if (!viewer) return;
    viewer.onHover = setHover;
    viewer.onClick = (h, e) => {
      const st = useStore.getState();
      if (st.tool === 'measure') {
        if (!h) return;
        if (!st.pending) st.set({ pending: h.point, picked: h });
        else st.set({ pending: null, picked: h, measurements: [...st.measurements, { id: newMeasureId(), a: st.pending, b: h.point }] });
      } else if (st.tool === 'features') {
        pickFeatureFace(h, e.shiftKey || e.metaKey || e.ctrlKey);
      } else {
        // no ordering of bodies in 3D, so shift adds/removes like cmd/ctrl
        selectBody(h ? h.body : null, e.shiftKey || e.metaKey || e.ctrlKey ? 'toggle' : 'one');
        st.set({ picked: h });
      }
    };
  }, [viewer]);

  const nBodies = s.app?.bodies.length ?? 0;
  useEffect(() => {
    viewer?.setTess(s.tess, nBodies);
  }, [viewer, s.tess, nBodies]);

  // faces picked for removal (after setTess, which drops the old highlight)
  const featureFaces = useFeatures((f) => f.faces);
  useEffect(() => viewer?.setFacePicking(s.tool === 'features'), [viewer, s.tool]);
  useEffect(() => viewer?.setFaceHighlight(featureFaces.map((f) => f.tag)), [viewer, featureFaces, s.tess]);

  // re-frame only when a new model is loaded (keep the camera across transforms)
  const saved = useRef<Partial<Record<ViewContext, { camera: CameraState; region: boolean }>>>({});
  /** orient along the trap axis once the first analysis arrives */
  const wantAxisView = useRef(false);
  useEffect(() => {
    if (viewer && useStore.getState().tess) {
      saved.current = {};
      viewer.view('iso');
    }
  }, [viewer, s.fitRequest]);

  // the trap tabs look at the trap region; each context keeps its own camera
  const ctx = useRef<ViewContext>(contextOf(s.tab));
  useEffect(() => {
    if (!viewer) return;
    const next = contextOf(s.tab);
    const prev = ctx.current;
    if (next === prev) return;
    ctx.current = next;
    const st = useStore.getState();
    saved.current[prev] = { camera: viewer.getCamera(), region: st.view.region };
    const back = saved.current[next];
    if (back) {
      st.setView({ region: back.region });
      viewer.setCamera(back.camera);
    } else if (next === 'trap' && st.tess) {
      // first visit: the electrodes around the trap center, down the trap axis
      st.setView({ region: true });
      st.set({ focusRequest: st.focusRequest + 1 });
      wantAxisView.current = true;
    } else st.setView({ region: false });
  }, [viewer, s.tab]);

  useEffect(() => {
    if (!viewer || !s.trap || !wantAxisView.current || contextOf(useStore.getState().tab) !== 'trap') return;
    wantAxisView.current = false;
    const r = trapRegion(useStore.getState());
    viewer.focus(r.center, r.half * 1.1, axisView(s.trap));
  }, [viewer, s.trap]);

  const region = s.view.region && s.tess ? trapRegion(s) : null;
  const regionKey = region ? JSON.stringify(region) : '';
  useEffect(() => {
    viewer?.setRegion(region);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewer, regionKey, s.tess, s.mesh]);
  useEffect(() => {
    if (!viewer || !s.focusRequest) return;
    const r = trapRegion(useStore.getState());
    viewer.focus(r.center, r.half * 1.1);
  }, [viewer, s.focusRequest]);

  // keyed on what the colors depend on: `bodies` is a new array on every state poll, and
  // new colors rebuild the whole BEM-mesh overlay
  const bodies = s.app?.bodies;
  const bodiesKey = (bodies ?? []).map((b) => `${b.name}\u0000${b.label?.electrode ?? ''}\u0000${b.label?.role ?? ''}`).join('\u0001');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const colors = useMemo(() => bodyColors(bodies ?? [], s.view.colorBy), [bodiesKey, s.view.colorBy]);

  useEffect(() => {
    viewer?.setBodyStyles(
      colors.map((color, i) => ({ color, visible: !s.hidden.includes(i), selected: s.selection.includes(i) })),
      s.view.xray,
      s.view.edges,
    );
  }, [viewer, colors, s.hidden, s.selection, s.view.xray, s.view.edges, s.tess]);

  // charge density of the selected basis solution on a signed log scale spanning
  // SIGMA_DECADES below the 99th percentile of |σ| (σ varies over decades and is
  // singular at sharp edges, so a linear scale shows almost nothing)
  const sigmaValues = useMemo(() => {
    const sg = s.sigma;
    // the charges belong to the mesh the solution was computed on
    if (!sg || !s.mesh || sg.nTri !== s.mesh.triBody.length || s.solution?.mesh_version !== s.app?.mesh?.version) return null;
    const b = Math.min(s.view.sigmaBasis, sg.nBasis - 1);
    const raw = sg.data.subarray(b * sg.nTri, (b + 1) * sg.nTri);
    const mags = Float64Array.from(raw, Math.abs).sort();
    const hi = mags[Math.floor(0.99 * (mags.length - 1))] || 1;
    const lo = hi * 10 ** -SIGMA_DECADES;
    const data = Float32Array.from(raw, (v) => {
      const a = Math.abs(v);
      return a <= lo ? 0 : (Math.sign(v) * Math.log10(a / lo)) / SIGMA_DECADES;
    });
    return { data, clip: 1, hi };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [s.sigma, s.mesh, s.view.sigmaBasis, s.solution?.mesh_version, s.app?.mesh?.version]);

  useEffect(() => {
    if (!viewer) return;
    const range = viewer.setSurfaceMesh(s.mesh, s.view.meshColor, colors, sigmaValues);
    useStore.setState({ meshSizeRange: range, sigmaClip: sigmaValues?.hi ?? null });
  }, [viewer, s.mesh, s.view.meshColor, colors, sigmaValues]);

  // the BEM mesh replaces the CAD bodies only where it is the subject: the Mesh tab, and
  // the Solve tab when it shows a charge density
  const overlay = s.view.showMesh && (s.tab === 'mesh' || (s.tab === 'solve' && s.view.meshColor === 'sigma'));
  useEffect(() => viewer?.setOverlayVisible(overlay), [viewer, overlay, s.mesh]);

  // trap analysis: equilibrium, RF null and principal axes
  useEffect(() => {
    const a = s.trap?.analysis;
    viewer?.setTrapMarkers(
      a && s.view.showModes
        ? {
            eq: a.equilibrium,
            rfNull: a.rf_null,
            axes: a.modes.map((m, i) => ({ dir: m.axis, color: MODE_COLORS[i] })),
            len: 0.4 * (s.trap?.r_valid ?? 1e-4),
            escape: s.trap?.depth ? { path: s.trap.depth.path, barrier: s.trap.depth.barrier_point } : undefined,
          }
        : null,
    );
  }, [viewer, s.trap, s.view.showModes]);

  useEffect(() => viewer?.setIons(s.view.showIons ? s.ions : null), [viewer, s.ions, s.view.showIons]);

  useEffect(() => {
    const sl = s.slice;
    viewer?.setSlice(
      sl && s.view.showSlice
        ? {
            center: sl.data.center,
            uAxis: sl.data.u_axis,
            vAxis: sl.data.v_axis,
            half: sl.data.half,
            n: sl.data.n,
            ...sliceImage(sl.data.values, sl.data.n, sl.ref, sl.clip),
          }
        : null,
    );
  }, [viewer, s.slice, s.view.showSlice]);
  useEffect(() => viewer?.setProjection(s.view.projection), [viewer, s.view.projection]);
  useEffect(() => viewer?.setRotateMode(s.view.rotate), [viewer, s.view.rotate]);
  useEffect(() => viewer?.setAxesVisible(s.view.axes), [viewer, s.view.axes]);
  useEffect(() => {
    const sec = s.view.section;
    viewer?.setSection(sec.axis, sec.offset, sec.flip);
  }, [viewer, s.view.section, s.tess, s.mesh]);

  useEffect(() => {
    viewer?.setMeasurements(
      s.measurements.map((m) => {
        const d = Math.hypot(m.b[0] - m.a[0], m.b[1] - m.a[1], m.b[2] - m.a[2]);
        return { a: m.a, b: m.b, label: len(d, s.units) };
      }),
      s.pending,
    );
  }, [viewer, s.measurements, s.pending, s.units, s.tess]);

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDrag(false);
    const f = e.dataTransfer.files[0];
    if (f) s.uploadStep(f);
  };

  return (
    <div
      className={`viewer ${s.tool === 'measure' || s.tool === 'features' ? 'measuring' : ''} ${drag ? 'dragging' : ''}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDrag(true);
      }}
      onDragLeave={() => setDrag(false)}
      onDrop={onDrop}
    >
      <div ref={host} className="viewer-host" />
      <ViewerToolbar />
      {!s.tess && !s.busy && <div className="viewer-empty">Open a STEP file from the Geometry panel, or drop one here.</div>}
      {drag && <div className="viewer-drop">Drop STEP file to load</div>}
    </div>
  );
}
