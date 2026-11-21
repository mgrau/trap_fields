import { useMemo, useState } from 'react';
import { api, ROLES, type Label, type Role } from '../api';
import { bodyColors, ROLE_COLORS, ROLE_NAMES } from '../colors';
import { selectBody, useStore } from '../store';
import { fmtArea, fmtVolume, len, vec } from '../units';
import { KV, Needs, NextStep, Section, Seg } from '../widgets';

export function ElectrodesPanel() {
  const s = useStore();
  const app = s.app;
  const u = s.units;
  const bodies = app?.bodies ?? [];
  const colors = useMemo(() => bodyColors(bodies, s.view.colorBy), [bodies, s.view.colorBy]);

  const cur = (name: string): Label => app?.project.labels[name] ?? { electrode: name, role: 'unassigned' };
  // one request for all bodies, so the electrodes are rebuilt once
  const save = (names: string[], patch: Partial<Label>) =>
    s.run('Saving labels', () => api.labels(Object.fromEntries(names.map((n) => [n, { ...cur(n), ...patch }]))));

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

  if (!app?.model) return <Needs tab="geometry">Load a STEP file first.</Needs>;
  const sel = s.selection.length === 1 && s.selected !== null ? bodies[s.selected] : null;
  const toggleHidden = (i: number) => s.set({ hidden: s.hidden.includes(i) ? s.hidden.filter((x) => x !== i) : [...s.hidden, i] });
  const chosen = s.selection.filter((i) => i < bodies.length);
  const allChosen = bodies.length > 0 && chosen.length === bodies.length;
  const chosenHidden = chosen.length > 0 && chosen.every((i) => s.hidden.includes(i));

  return (
    <div className="panel">
      <Section
        title="Bodies"
        info="Click a body in the 3D view or a row here; shift-click a row selects a range, cmd/ctrl-click (or shift-click in 3D) adds or removes one. With several selected, assign their role or electrode name together. Bodies with the same electrode name are electrically connected (one electrode). Roles: RF electrodes get the RF drive by default, DC ones static voltages, ground stays at 0 V, floating electrodes carry zero net charge."
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
        <table className="bodies">
          <thead>
            <tr>
              <th>
                <input
                  type="checkbox"
                  title={allChosen ? 'Select none' : 'Select all'}
                  checked={allChosen}
                  ref={(el) => {
                    if (el) el.indeterminate = chosen.length > 0 && !allChosen;
                  }}
                  onChange={() =>
                    allChosen ? selectBody(null) : s.set({ selection: bodies.map((_, i) => i), selected: s.selected ?? 0 })
                  }
                />
              </th>
              <th />
              <th>Body</th>
              <th>Electrode</th>
              <th>Role</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {bodies.map((b, i) => (
              <tr
                key={b.name}
                className={chosen.includes(i) ? 'sel' : ''}
                // keep shift-click from selecting the table's text
                onMouseDown={(e) => e.shiftKey && e.preventDefault()}
                onClick={(e) => selectBody(i, e.shiftKey ? 'range' : e.metaKey || e.ctrlKey ? 'toggle' : 'one')}
              >
                <td>
                  <input type="checkbox" checked={chosen.includes(i)} onClick={(e) => e.stopPropagation()} onChange={() => selectBody(i, 'toggle')} />
                </td>
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
                      if (v && v !== b.label?.electrode) save([b.name], { electrode: v });
                      else e.target.value = b.label?.electrode ?? b.name;
                    }}
                    onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
                  />
                </td>
                <td>
                  <select
                    value={b.label?.role ?? 'unassigned'}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => save([b.name], { role: e.target.value as Role })}
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
        {chosen.length > 1 && (
          <BulkAssign
            // a new selection starts from its own common values
            key={chosen.join(',')}
            labels={chosen.map((i) => cur(bodies[i].name))}
            onSave={(patch) => save(chosen.map((i) => bodies[i].name), patch)}
            onClear={() => selectBody(null)}
          />
        )}
        <div className="row">
          <button disabled={!chosen.length} onClick={() => s.set({ hidden: bodies.map((_, i) => i).filter((i) => !chosen.includes(i)) })}>
            Isolate selected
          </button>
          <button
            disabled={!chosen.length}
            onClick={() => s.set({ hidden: chosenHidden ? s.hidden.filter((i) => !chosen.includes(i)) : [...new Set([...s.hidden, ...chosen])] })}
          >
            {chosenHidden ? 'Show selected' : 'Hide selected'}
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
      {app.bodies.every((b) => (b.label?.role ?? 'unassigned') !== 'unassigned') && <NextStep tab="mesh" label="surface mesh" />}
    </div>
  );
}

/** Role and electrode name for several bodies at once. */
function BulkAssign({ labels, onSave, onClear }: { labels: Label[]; onSave: (patch: Partial<Label>) => void; onClear: () => void }) {
  const roles = new Set(labels.map((l) => l.role));
  const role = roles.size === 1 ? labels[0].role : '';
  const names = new Set(labels.map((l) => l.electrode));
  const [name, setName] = useState(labels[0].electrode);
  const group = () => {
    const v = name.trim();
    if (v) onSave({ electrode: v });
  };
  return (
    <div className="bulk-assign">
      <div className="bulk-head">
        <b>{labels.length} bodies selected</b>
        <button className="link" onClick={onClear} title="Clear the selection (Esc)">
          clear
        </button>
      </div>
      <label>
        Role
        <select
          value={role}
          onChange={(e) => onSave({ role: e.target.value as Role })}
          style={{ borderColor: role ? ROLE_COLORS[role] : undefined }}
        >
          {!role && (
            <option value="" disabled>
              mixed
            </option>
          )}
          {ROLES.map((r) => (
            <option key={r} value={r}>
              {ROLE_NAMES[r]}
            </option>
          ))}
        </select>
      </label>
      <label>
        Electrode
        <input value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && group()} />
        <button
          disabled={!name.trim() || (names.size === 1 && labels[0].electrode === name.trim())}
          title="Connect the selected bodies as one electrode with this name"
          onClick={group}
        >
          Group
        </button>
      </label>
    </div>
  );
}
