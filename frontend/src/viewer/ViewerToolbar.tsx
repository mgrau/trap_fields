// Controls of the 3D view, floating over it: camera, display, section, measuring and the
// trap-region clip box. They live with the view so they disappear with it.
import { useStore, type SectionAxis } from '../store';
import { num, unitLabel } from '../units';
import { LengthField, Seg } from '../widgets';
import type { ViewDir } from './Viewer';
import { trapRegion, viewerCommands } from './commands';

const VIEWS: { d: ViewDir; label: string }[] = [
  { d: 'iso', label: 'Iso' },
  { d: '+x', label: '+X' },
  { d: '-x', label: '−X' },
  { d: '+y', label: '+Y' },
  { d: '-y', label: '−Y' },
  { d: '+z', label: '+Z' },
  { d: '-z', label: '−Z' },
];

export function ViewerToolbar() {
  const s = useStore();
  const v = s.view;
  const bb = s.app?.bbox;
  const sec = v.section;
  const axisIdx = sec.axis ? { x: 0, y: 1, z: 2 }[sec.axis] : 0;
  const lo = bb ? bb[axisIdx] : -1;
  const hi = bb ? bb[axisIdx + 3] : 1;
  if (!s.tess) return null;
  return (
    <div className="viewer-toolbar" onPointerDown={(e) => e.stopPropagation()}>
      <div className="vt-group">
        <select
          value=""
          title="View from a direction"
          onChange={(e) => {
            if (e.target.value) viewerCommands.view(e.target.value as ViewDir);
          }}
        >
          <option value="">View…</option>
          {VIEWS.map((x) => (
            <option key={x.d} value={x.d}>
              {x.d === 'iso' ? 'Isometric' : `From ${x.label}`}
            </option>
          ))}
        </select>
        <button onClick={() => viewerCommands.fit()} title="Fit everything in view (F)">
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
      <div className="vt-group">
        <button className={v.edges ? 'on' : ''} aria-pressed={v.edges} onClick={() => s.setView({ edges: !v.edges })} title="Show CAD edges">
          Edges
        </button>
        <button className={v.xray ? 'on' : ''} aria-pressed={v.xray} onClick={() => s.setView({ xray: !v.xray })} title="Semi-transparent bodies">
          X-ray
        </button>
        <button className={v.axes ? 'on' : ''} aria-pressed={v.axes} onClick={() => s.setView({ axes: !v.axes })} title="Origin axes (X red, Y green, Z blue)">
          Axes
        </button>
      </div>
      <div className="vt-group" title="Section plane">
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
      <button className={s.tool === 'measure' ? 'on' : ''} aria-pressed={s.tool === 'measure'} onClick={() => s.set({ tool: s.tool === 'measure' ? 'select' : 'measure', pending: null })} title="Measure distance (M)">
        Measure
      </button>
      <div className="vt-group" title="Show only the electrodes in a box around the trap center">
        <button
          className={v.region ? 'on' : ''} aria-pressed={v.region}
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
    </div>
  );
}
