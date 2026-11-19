import { useMemo } from 'react';
import { api, ROLES, type Label, type Role } from '../api';
import { bodyColors, ROLE_COLORS, ROLE_NAMES } from '../colors';
import { useStore } from '../store';
import { fmtArea, fmtVolume, len, vec } from '../units';
import { KV, Section, Seg } from '../widgets';

export function ElectrodesPanel() {
  const s = useStore();
  const app = s.app;
  const u = s.units;
  const bodies = app?.bodies ?? [];
  const colors = useMemo(() => bodyColors(bodies, s.view.colorBy), [bodies, s.view.colorBy]);

  const save = (name: string, patch: Partial<Label>) => {
    const cur = app?.project.labels[name] ?? { electrode: name, role: 'unassigned' as Role };
    s.run('Saving labels', () => api.labels({ [name]: { ...cur, ...patch } }));
  };

  const electrodes = useMemo(() => {
    const m = new Map<string, { role: Role; bodies: string[]; roles: Set<Role> }>();
    for (const b of bodies) {
      const l = b.label ?? { electrode: b.name, role: 'unassigned' as Role };
      const e = m.get(l.electrode) ?? { role: l.role, bodies: [], roles: new Set<Role>() };
      e.bodies.push(b.name);
      e.roles.add(l.role);
      m.set(l.electrode, e);
    }
    return [...m.entries()];
  }, [bodies]);

  if (!app?.model) return <div className="panel hint pad">Load a STEP file first.</div>;
  const sel = s.selected !== null ? bodies[s.selected] : null;
  const toggleHidden = (i: number) => s.set({ hidden: s.hidden.includes(i) ? s.hidden.filter((x) => x !== i) : [...s.hidden, i] });

  return (
    <div className="panel">
      <Section
        title="Bodies"
        right={
          <Seg
            value={s.view.colorBy}
            onChange={(colorBy) => s.setView({ colorBy })}
            options={[
              { value: 'electrode', label: 'by electrode' },
              { value: 'role', label: 'by role' },
            ]}
          />
        }
      >
        <p className="hint">
          Click a body in the viewport or a row here. Bodies given the same electrode name are electrically connected (one electrode).
        </p>
        <table className="bodies">
          <thead>
            <tr>
              <th />
              <th>Body</th>
              <th>Electrode</th>
              <th>Role</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {bodies.map((b, i) => (
              <tr key={b.name} className={s.selected === i ? 'sel' : ''} onClick={() => s.set({ selected: i })}>
                <td>
                  <span className="swatch" style={{ background: colors[i] }} />
                </td>
                <td title={b.step_name !== b.name ? `STEP name: ${b.step_name}` : undefined}>{b.name}</td>
                <td>
                  <input
                    key={b.label?.electrode}
                    defaultValue={b.label?.electrode ?? b.name}
                    onClick={(e) => e.stopPropagation()}
                    onBlur={(e) => {
                      const v = e.target.value.trim();
                      if (v && v !== b.label?.electrode) save(b.name, { electrode: v });
                      else e.target.value = b.label?.electrode ?? b.name;
                    }}
                    onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
                  />
                </td>
                <td>
                  <select
                    value={b.label?.role ?? 'unassigned'}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => save(b.name, { role: e.target.value as Role })}
                    style={{ borderColor: ROLE_COLORS[b.label?.role ?? 'unassigned'] }}
                  >
                    {ROLES.map((r) => (
                      <option key={r} value={r}>
                        {ROLE_NAMES[r]}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <button
                    className="icon"
                    title={s.hidden.includes(i) ? 'Show' : 'Hide'}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleHidden(i);
                    }}
                  >
                    {s.hidden.includes(i) ? '◌' : '●'}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="row">
          <button disabled={s.selected === null} onClick={() => s.set({ hidden: bodies.map((_, i) => i).filter((i) => i !== s.selected) })}>
            Isolate selected
          </button>
          <button disabled={s.selected === null} onClick={() => s.selected !== null && toggleHidden(s.selected)}>
            Hide selected
          </button>
          <button disabled={!s.hidden.length} onClick={() => s.set({ hidden: [] })}>
            Show all
          </button>
        </div>
      </Section>

      <Section title="Electrodes">
        {electrodes.map(([name, e]) => (
          <div key={name} className="electrode">
            <span className="role-dot" style={{ background: ROLE_COLORS[e.role] }} />
            <b>{name}</b>
            <span className="hint">
              {e.roles.size > 1 ? 'mixed roles!' : ROLE_NAMES[e.role]} · {e.bodies.length} {e.bodies.length === 1 ? 'body' : 'bodies'}
            </span>
          </div>
        ))}
        {electrodes.some(([, e]) => e.roles.size > 1) && <div className="warn">Bodies of one electrode have different roles.</div>}
        {bodies.some((b) => (b.label?.role ?? 'unassigned') === 'unassigned') && (
          <div className="hint">Assign a role to every body before solving.</div>
        )}
      </Section>

      {sel && (
        <Section title={`Selected: ${sel.name}`}>
          <KV k="STEP name" v={sel.step_name || '—'} />
          <KV k="Faces" v={sel.n_faces} />
          <KV k="Volume" v={fmtVolume(sel.volume)} />
          <KV k="Surface area" v={fmtArea(sel.area)} />
          {sel.bbox && (
            <>
              <KV k="Size X" v={len(sel.bbox[3] - sel.bbox[0], u)} />
              <KV k="Size Y" v={len(sel.bbox[4] - sel.bbox[1], u)} />
              <KV k="Size Z" v={len(sel.bbox[5] - sel.bbox[2], u)} />
            </>
          )}
          {sel.closest && (
            <>
              <KV k="Distance to origin" v={len(sel.closest.distance, u)} title="Closest point of this body's surface to the origin of the current frame" />
              <KV k="Closest point" v={vec(sel.closest.point, u)} />
            </>
          )}
          {sel.name_match_cost !== null && (
            <KV k="Name match" v={`bbox Δ ${len(sel.name_match_cost, 'um', 1)}`} title="Bounding-box mismatch between the STEP solid and the imported volume" />
          )}
        </Section>
      )}
    </div>
  );
}
