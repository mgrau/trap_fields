import { useEffect, useMemo, useRef, useState } from 'react';

export const SIGMA_DECADES = 3;
import { api } from '../api';
import { MODE_COLORS, bodyColors, sliceRgba } from '../colors';
import { newMeasureId, useStore } from '../store';
import { len } from '../units';
import { Viewer, type ViewDir } from './Viewer';

let viewerRef: Viewer | null = null;
/** Imperative camera commands for the toolbar. */
export const viewerCommands = {
  view: (d: ViewDir) => viewerRef?.view(d),
  fit: () => viewerRef?.fit(),
};

export function ViewerCanvas() {
  const host = useRef<HTMLDivElement>(null);
  const [viewer, setViewer] = useState<Viewer | null>(null);
  const [drag, setDrag] = useState(false);
  const s = useStore();

  useEffect(() => {
    const v = new Viewer(host.current!);
    viewerRef = v;
    setViewer(v);
    return () => {
      v.dispose();
      viewerRef = null;
    };
  }, []);

  // pointer callbacks read the latest store state
  useEffect(() => {
    if (!viewer) return;
    viewer.onHover = (h) => useStore.getState().set({ hover: h });
    viewer.onClick = (h) => {
      const st = useStore.getState();
      if (st.tool === 'measure') {
        if (!h) return;
        if (!st.pending) st.set({ pending: h.point, picked: h });
        else st.set({ pending: null, picked: h, measurements: [...st.measurements, { id: newMeasureId(), a: st.pending, b: h.point }] });
      } else {
        st.set({ selected: h ? h.body : null, picked: h });
      }
    };
  }, [viewer]);

  const nBodies = s.app?.bodies.length ?? 0;
  useEffect(() => {
    viewer?.setTess(s.tess, nBodies);
  }, [viewer, s.tess, nBodies]);

  // re-frame only when a new model is loaded (keep the camera across transforms)
  useEffect(() => {
    if (viewer && useStore.getState().tess) viewer.view('iso');
  }, [viewer, s.fitRequest]);

  const colors = useMemo(() => bodyColors(s.app?.bodies ?? [], s.view.colorBy), [s.app?.bodies, s.view.colorBy]);

  useEffect(() => {
    viewer?.setBodyStyles(
      colors.map((color, i) => ({ color, visible: !s.hidden.includes(i), selected: s.selected === i })),
      s.view.xray,
      s.view.edges,
    );
  }, [viewer, colors, s.hidden, s.selected, s.view.xray, s.view.edges, s.tess]);

  // charge density of the selected basis solution on a signed log scale spanning
  // SIGMA_DECADES below the 99th percentile of |σ| (σ varies over decades and is
  // singular at sharp edges, so a linear scale shows almost nothing)
  const sigmaValues = useMemo(() => {
    const sg = s.sigma;
    if (!sg || !s.mesh || sg.nTri !== s.mesh.triBody.length) return null;
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
  }, [s.sigma, s.mesh, s.view.sigmaBasis]);

  useEffect(() => {
    if (!viewer) return;
    const range = viewer.setSurfaceMesh(s.mesh, s.view.meshColor, colors, sigmaValues);
    useStore.setState({ meshSizeRange: range, sigmaClip: sigmaValues?.hi ?? null });
  }, [viewer, s.mesh, s.view.meshColor, colors, sigmaValues]);

  useEffect(() => viewer?.setOverlayVisible(s.view.showMesh), [viewer, s.view.showMesh, s.mesh]);

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
          }
        : null,
    );
  }, [viewer, s.trap, s.view.showModes]);

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
            rgba: sliceRgba(sl.data.values, sl.data.n, sl.ref, sl.clip),
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
    if (f) s.run(`Uploading ${f.name}`, () => api.upload(f));
  };

  return (
    <div
      className={`viewer ${s.tool === 'measure' ? 'measuring' : ''} ${drag ? 'dragging' : ''}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDrag(true);
      }}
      onDragLeave={() => setDrag(false)}
      onDrop={onDrop}
    >
      <div ref={host} className="viewer-host" />
      {!s.tess && !s.busy && <div className="viewer-empty">Open a STEP file from the Geometry panel, or drop one here.</div>}
      {drag && <div className="viewer-drop">Drop STEP file to load</div>}
    </div>
  );
}
