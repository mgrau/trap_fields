import { useEffect, useMemo, useState } from 'react';
import { api, type SolveParams, type SolveSummary, type Vec3 } from '../api';
import { ROLE_COLORS, ROLE_NAMES, divergingCss } from '../colors';
import { useStore } from '../store';
import { len, vec } from '../units';
import { SIGMA_DECADES } from '../viewer/ViewerCanvas';
import { KV, LengthField, NumberField, Section, Seg } from '../widgets';

/** Compact number with n significant figures. */
export function sig(x: number, n = 4): string {
  if (!Number.isFinite(x)) return '—';
  if (x === 0) return '0';
  const a = Math.abs(x);
  return a >= 1e-3 && a < 1e5 ? Number(x.toPrecision(n)).toString() : x.toExponential(n - 1);
}

const idx = (l: number, m: number) => l * l + l + m;
/** "auto" switches to the compressed solver from this many panels (matches the server). */
const AUTO_COMPRESSED_FROM = 5000;

export function SolvePanel() {
  const s = useStore();
  const app = s.app;
  const u = s.units;
  const [p, setP] = useState<SolveParams | null>(null);
  const stepFile = app?.project.step_file;
  useEffect(() => setP(null), [stepFile]);
  useEffect(() => {
    if (!p && app) setP(app.project.solve_params ?? { lmax: 10, center: null, r0: null });
  }, [app, p]);

  if (!app?.model) return <div className="panel hint pad">Load a STEP file first.</div>;
  if (!p) return null;

  const mesh = app.mesh;
  const job = app.job;
  const solving = job.state === 'running' && job.kind === 'solve';
  const n = mesh?.stats.n_triangles ?? 0;
  const defects = mesh?.stats.defective_faces ?? [];
  const electrodes = new Set(app.bodies.map((b) => b.label?.electrode ?? b.name));
  const floating = new Set(app.bodies.filter((b) => b.label?.role === 'floating').map((b) => b.label!.electrode));
  const unassigned = app.bodies.filter((b) => (b.label?.role ?? 'unassigned') === 'unassigned').length;
  const method = p.method ?? 'auto';
  const compressed = method === 'compressed' || (method === 'auto' && n >= AUTO_COMPRESSED_FROM);
  const gb = (8 * n * n) / 1e9;
  const center: Vec3 = p.center ?? mesh?.params.center ?? [0, 0, 0];
  const limit = compressed ? 400000 : 60000;
  const canSolve = !!mesh && defects.length === 0 && !solving && !s.busy && n <= limit;

  return (
    <div className="panel">
      <Section title="Boundary-value problem">
        <p className="hint">
          Free-space BEM (collocation, constant charge density per panel). One basis solution per electrode: 1 V on it, 0 V on the others; floating
          electrodes carry zero net charge. Each potential is then expanded in solid harmonics about the center, directly from the surface charges.
        </p>
        <ul className="checklist">
          <li className={mesh ? 'ok' : ''}>Surface mesh{mesh ? ` (${n.toLocaleString()} panels)` : ' — generate one on the Mesh tab'}</li>
          {mesh && <li className={defects.length === 0 ? 'ok' : ''}>{defects.length === 0 ? 'Mesh free of folds and duplicates' : `Mesh defects on faces ${defects.join(', ')}`}</li>}
          <li className="ok">
            {electrodes.size} electrode{electrodes.size === 1 ? '' : 's'}
            {floating.size ? `, ${floating.size} floating` : ''}
          </li>
          <li className={unassigned === 0 ? 'ok' : ''}>
            Roles {unassigned === 0 ? 'assigned' : `not assigned for ${unassigned} bodies (not needed for the basis solutions)`}
          </li>
        </ul>

        <h4>Expansion</h4>
        <div className="row">
          {(['x', 'y', 'z'] as const).map((l, i) => (
            <LengthField key={l} label={l} value={center[i]} units={u} onChange={(m) => setP({ ...p, center: center.map((x, j) => (j === i ? m : x)) as Vec3 })} />
          ))}
        </div>
        <div className="row">
          <button onClick={() => setP({ ...p, center: null })} title="Use the mesh refinement center">
            Mesh center
          </button>
          <button disabled={!s.picked} onClick={() => s.picked && setP({ ...p, center: s.picked.point })}>
            Picked point
          </button>
        </div>
        <div className="grid2">
          <NumberField label="l_max" value={p.lmax} onChange={(v) => setP({ ...p, lmax: Math.max(1, Math.min(40, Math.round(v))) })} title="Highest solid-harmonic order" />
          <div className="row">
            <label className="check" title="r0 = distance from the center to the nearest electrode">
              <input type="checkbox" checked={p.r0 === null} onChange={(e) => setP({ ...p, r0: e.target.checked ? null : (s.solution?.r0 ?? 1e-3) })} />
              auto r0
            </label>
          </div>
          {p.r0 !== null && <LengthField label="r0" value={p.r0} units={u} onChange={(r0) => setP({ ...p, r0 })} title="Length scale of the expansion coefficients" />}
        </div>
        <div className="row">
          <span className="hint">Linear solver</span>
          <Seg
            value={method}
            onChange={(m) => setP({ ...p, method: m })}
            options={[
              { value: 'auto', label: 'auto', title: `Compressed from ${AUTO_COMPRESSED_FROM.toLocaleString()} panels on, dense below` },
              { value: 'dense', label: 'dense LU', title: 'Full matrix and LU factorization: exact, memory and time grow as N² and N³' },
              { value: 'compressed', label: 'compressed', title: 'H-matrix (ACA) far field + GMRES: agrees with dense to ~1e-7, far less memory and time' },
            ]}
          />
        </div>
        {mesh && (
          <p className="hint">
            {n.toLocaleString()} unknowns,{' '}
            {compressed
              ? `compressed solver (H-matrix + GMRES), roughly ${(gb * 0.25 < 1 ? `${(gb * 250).toFixed(0)} MB` : `${(gb * 0.25).toFixed(1)} GB`)} or less.`
              : `dense matrix ${gb < 1 ? `${(gb * 1000).toFixed(0)} MB` : `${gb.toFixed(1)} GB`}; LU time grows as n³.`}
            {n > limit && <span className="error"> Too many panels for this solver (limit {limit.toLocaleString()}).</span>}
          </p>
        )}
        <div className="row">
          <button className="primary" disabled={!canSolve} onClick={() => s.run('Starting solver', () => api.solve(p))}>
            {solving ? `Solving: ${job.stage ?? ''} ${(job.elapsed ?? 0).toFixed(0)} s` : 'Solve'}
          </button>
        </div>
        {job.state === 'failed' && job.kind === 'solve' && <div className="error">{job.error}</div>}
      </Section>

      {s.solution && <Results sol={s.solution} />}
    </div>
  );
}

function Results({ sol }: { sol: SolveSummary }) {
  const s = useStore();
  const u = s.units;
  const app = s.app!;
  const [b, setB] = useState(0);
  const [allCoeffs, setAllCoeffs] = useState(false);
  const basis = sol.basis[Math.min(b, sol.basis.length - 1)];
  const roles = useMemo(() => {
    const r: Record<string, string> = {};
    for (const body of app.bodies) if (body.label) r[body.label.electrode] ??= body.label.role;
    return r;
  }, [app.bodies]);
  const showing = s.view.showMesh && s.view.meshColor === 'sigma';
  const show = (bi: number, on: boolean) =>
    s.setView(on ? { showMesh: true, meshColor: 'sigma', sigmaBasis: bi } : { meshColor: 'size' });
  const t = sol.timings;
  const L = basis.local;
  const hNorm = Math.hypot(...L.grad);
  const lNorms = Array.from({ length: sol.lmax + 1 }, (_, l) => Math.hypot(...basis.coeffs.slice(l * l, (l + 1) * (l + 1))));
  const lMax = Math.max(...lNorms.filter((x) => x > 0));
  const nShow = allCoeffs ? sol.lmax : Math.min(sol.lmax, 4);

  return (
    <>
      <Section title="Solution">
        {sol.stale && <div className="warn">Electrode grouping or floating roles changed since this solve. Solve again.</div>}
        <KV k="Panels" v={`${sol.n_panels.toLocaleString()} (${sol.n_unknowns.toLocaleString()} unknowns)`} />
        {sol.solver?.method === 'compressed' ? (
          <>
            <KV
              k="Time"
              v={`${sol.seconds.toFixed(1)} s (H-matrix ${t.assemble.toFixed(2)}, preconditioner ${t.factor.toFixed(2)}, GMRES ${t.solve.toFixed(2)}, expand ${sol.expand_seconds.toFixed(2)})`}
            />
            <KV
              k="Compressed matrix"
              v={`${(sol.solver.matrix_bytes / 1e6).toFixed(0)} MB (${((sol.solver.compression ?? 0) * 100).toFixed(1)} % of dense), max rank ${sol.solver.max_rank}`}
            />
            <KV
              k="GMRES"
              v={`${Math.max(...sol.solver.iterations)} iterations, residual ≤ ${sig(Math.max(...sol.solver.residual), 2)}`}
            />
          </>
        ) : (
          <KV k="Time" v={`${sol.seconds.toFixed(1)} s (assemble ${t.assemble.toFixed(2)}, LU ${t.factor.toFixed(1)}, solve ${t.solve.toFixed(2)}, expand ${sol.expand_seconds.toFixed(2)})`} />
        )}
        <KV k="Center" v={vec(sol.center, u)} />
        <KV k="r0" v={len(sol.r0, u)} />
        <KV k="Converges within" v={len(sol.r_min, u)} title="Distance from the center to the nearest panel: the expansion is valid inside this sphere" />
        <table className="bodies compact">
          <thead>
            <tr>
              <th>Electrode</th>
              <th title="Potential at the center for 1 V on this electrode">φ₀ (V/V)</th>
              <th title="|∇φ| at the center for 1 V">|∇φ| (V/m)</th>
              <th title="Self-capacitance (diagonal of the capacitance matrix)">C (fF)</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {sol.basis.map((x, i) => {
              const role = (roles[x.electrode] ?? 'unassigned') as keyof typeof ROLE_COLORS;
              return (
                <tr key={x.electrode} className={i === b ? 'sel' : ''} onClick={() => setB(i)}>
                  <td>
                    <span className="electrode">
                      <span className="role-dot" style={{ background: ROLE_COLORS[role] }} title={ROLE_NAMES[role]} />
                      {x.electrode}
                    </span>
                  </td>
                  <td className="num">{sig(x.local.phi, 5)}</td>
                  <td className="num">{sig(Math.hypot(...x.local.grad), 4)}</td>
                  <td className="num">{sig(x.charge[x.index] * 1e15, 5)}</td>
                  <td>
                    <input type="checkbox" title="Show charge density" checked={showing && s.view.sigmaBasis === i} onChange={(e) => show(i, e.target.checked)} onClick={(e) => e.stopPropagation()} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {sol.electrodes.some((e) => e.floating) && (
          <p className="hint">Floating: {sol.electrodes.filter((e) => e.floating).map((e) => e.name).join(', ')} (zero net charge, potential solved for).</p>
        )}
        {showing && s.sigmaClip && <SigmaLegend clip={s.sigmaClip} electrode={sol.basis[s.view.sigmaBasis]?.electrode} />}
      </Section>

      <Section title={`Basis: ${basis.electrode} at 1 V`}>
        <p className="hint">Local expansion at the center, per volt on {basis.electrode}. The field is E = −∇φ.</p>
        <KV k="φ" v={`${sig(L.phi, 6)} V`} />
        <KV k="∇φ" v={`(${L.grad.map((g) => sig(g, 5)).join(', ')}) V/m`} />
        <KV k="|∇φ|" v={`${sig(hNorm, 5)} V/m`} />
        <h4>Curvature ∂²φ/∂xᵢ∂xⱼ (V/mm²)</h4>
        <table className="bodies compact mono">
          <tbody>
            {L.hessian.map((row, i) => (
              <tr key={i}>
                <td className="hint">{'xyz'[i]}</td>
                {row.map((h, j) => (
                  <td key={j} className="num">
                    {sig(h * 1e-6, 4)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>

        <h4>Solid-harmonic coefficients a_lm (V), r0 = {len(sol.r0, u)}</h4>
        <p className="hint">φ(r) = Σ a_lm Y_lm((r − c)/r0); real, Racah-normalized: Y₁ = (x, y, z) for m = (1, −1, 0), Y₂,₀ = z² − (x²+y²)/2, …</p>
        <div className="lnorms" title="‖a_l‖ = sqrt(Σ_m a_lm²), log scale over 4 decades">
          {lNorms.map((v, l) => (
            <div key={l} className="row" title={`‖a_${l}‖ = ${sig(v)} V`}>
              <span className="mono small lbl">l={l}</span>
              <div className="track">
                <div className="bar" style={{ width: `${v > 0 ? Math.max(1, 100 * (1 + Math.log10(v / lMax) / 4)) : 0}%` }} />
              </div>
              <span className="mono small">{sig(v, 3)}</span>
            </div>
          ))}
        </div>
        <table className="bodies compact">
          <thead>
            <tr>
              <th>l</th>
              <th>m</th>
              <th>a_lm (V)</th>
              <th>l</th>
              <th>m</th>
              <th>a_lm (V)</th>
            </tr>
          </thead>
          <tbody>
            {pairs(nShow).map((row, i) => (
              <tr key={i}>
                {row.map(([l, m], j) =>
                  l < 0 ? (
                    <td key={j} colSpan={3} />
                  ) : (
                    <Cell key={j} l={l} m={m} v={basis.coeffs[idx(l, m)]} />
                  ),
                )}
              </tr>
            ))}
          </tbody>
        </table>
        {sol.lmax > 4 && (
          <button onClick={() => setAllCoeffs(!allCoeffs)}>{allCoeffs ? 'Show l ≤ 4' : `Show all (l ≤ ${sol.lmax})`}</button>
        )}
      </Section>

      <Section title="Capacitance matrix (fF)" defaultOpen={false}>
        <p className="hint">Row: electrode at 1 V; column: induced charge. Symmetric up to discretization error.</p>
        <div style={{ overflowX: 'auto' }}>
          <table className="bodies compact">
            <thead>
              <tr>
                <th />
                {sol.electrodes.map((e) => (
                  <th key={e.name}>{e.name}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {sol.basis.map((x) => (
                <tr key={x.electrode}>
                  <td>{x.electrode}</td>
                  {x.charge.map((q, k) => (
                    <td key={k} className="num">
                      {(q * 1e15).toFixed(2)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Section>

      <Section title="Output files" defaultOpen={false}>
        <p className="hint">
          solve/solution.json: expansions, local fields, capacitance matrix and the conventions. solve/sigma.bin: charge density per panel and basis
          (triangle order of mesh/surface_mesh.bin).
        </p>
      </Section>
    </>
  );
}

/** (l, m) pairs for l ≤ lmax laid out two per row. */
function pairs(lmax: number): [number, number][][] {
  const all: [number, number][] = [];
  for (let l = 0; l <= lmax; l++) for (let m = -l; m <= l; m++) all.push([l, m]);
  const rows: [number, number][][] = [];
  for (let i = 0; i < all.length; i += 2) rows.push([all[i], all[i + 1] ?? [-1, 0]]);
  return rows;
}

function Cell({ l, m, v }: { l: number; m: number; v: number }) {
  return (
    <>
      <td className="num hint">{l}</td>
      <td className="num hint">{m}</td>
      <td className="num">{sig(v, 4)}</td>
    </>
  );
}

function SigmaLegend({ clip, electrode }: { clip: number; electrode?: string }) {
  const stops = [-1, -0.5, 0, 0.5, 1].map(divergingCss).join(', ');
  const lo = clip * 10 ** -SIGMA_DECADES;
  return (
    <div className="legend">
      <div className="hint">Surface charge density σ, {electrode} at 1 V (signed log scale, nC/m²)</div>
      <div className="ramp" style={{ background: `linear-gradient(90deg, ${stops})` }} />
      <div className="row spread mono small">
        <span>−{sig(clip * 1e9, 2)}</span>
        <span>±{sig(lo * 1e9, 2)}</span>
        <span>+{sig(clip * 1e9, 2)}</span>
      </div>
    </div>
  );
}
