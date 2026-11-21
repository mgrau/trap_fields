// How far along the running job is, for the progress bar: a fraction only where one can
// be computed honestly from what the engine reports, else null (indeterminate).
import { DEFAULT_SOLVER_SETTINGS, type AppStateDto } from './api';

/**
 * GMRES: the relative residual falls from 1 to the tolerance roughly geometrically, so
 * progress is log(residual)/log(tolerance). All right-hand sides (one per electrode)
 * iterate together; the slowest one decides when the solve ends.
 */
function gmresFraction(gmres: [number, number[]][] | undefined, tol: number): number | null {
  const last = gmres?.[gmres.length - 1]?.[1];
  if (!last?.length || !(tol > 0 && tol < 1)) return null;
  const f = Math.min(...last.map((r) => (r > 0 ? Math.log(r) / Math.log(tol) : 1)));
  return Math.min(1, Math.max(0, f));
}

export function jobFraction(app: AppStateDto | null): number | null {
  const job = app?.job;
  if (!app || job?.state !== 'running') return null;
  const p = app.progress;
  const stage = job.stage ?? '';
  const tol = (app.project.solve_params?.settings ?? DEFAULT_SOLVER_SETTINGS).gmres_tol;
  if (job.kind === 'study') {
    // "level i/n: …"; within a level only the GMRES part advances the bar
    const m = /^level (\d+)\/(\d+)/.exec(stage);
    if (!m) return null;
    const [i, n] = [Number(m[1]) - 1, Number(m[2])];
    const inner = /GMRES/.test(stage) ? (gmresFraction(p?.gmres, tol) ?? 0) : 0;
    return n > 0 ? Math.min(1, (i + inner) / n) : null;
  }
  // a stage that counts its items (bodies, faces)
  if (p?.step && p.step[1] > 0) return Math.min(1, p.step[0] / p.step[1]);
  if (job.kind === 'solve' && stage === 'GMRES') return gmresFraction(p?.gmres, tol);
  return null;
}
