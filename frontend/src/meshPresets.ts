// Mesh fineness presets: the suggested sizes (app.mesh_defaults) scaled. Convergence
// studies of the four-blade trap: Normal 25k panels (axial frequency ~2 % high), Fine
// 36k (~1.5 %), Very fine 59k (~1.2 %); the panel count grows about as s^-1.25.
import type { AppStateDto, MeshParams } from './api';

export const FINENESS = [
  { key: 'coarse', label: 'Coarse', scale: 1.4 },
  { key: 'normal', label: 'Normal', scale: 1 },
  { key: 'fine', label: 'Fine', scale: 0.7 },
  { key: 'vfine', label: 'Very fine', scale: 0.5 },
] as const;

/** Fineness used when a project has no mesh settings yet. */
export const DEFAULT_SCALE = 0.7;

const r3 = (x: number) => Number(x.toPrecision(3));

export function scaledMesh(base: MeshParams, scale: number, center = base.center): MeshParams {
  return { ...base, center, h_min: r3(base.h_min * scale), h_max: r3(base.h_max * scale) };
}

/** Scale of `p` relative to `base` if it is a preset (same grading radii and curve points), else null. */
export function presetScale(p: MeshParams, base: MeshParams | null): number | null {
  if (!base || p.dist_min !== base.dist_min || p.dist_max !== base.dist_max || p.min_circle_points !== base.min_circle_points) return null;
  const f = FINENESS.find((f) => Math.abs(r3(base.h_min * f.scale) - p.h_min) <= 1e-9 * base.h_min && Math.abs(r3(base.h_max * f.scale) - p.h_max) <= 1e-9 * base.h_max);
  return f ? f.scale : null;
}

/** The project's mesh settings, else the default fineness of the suggested sizes. */
export function initialMesh(app: AppStateDto): MeshParams | null {
  return app.project.mesh_params ?? (app.mesh_defaults ? scaledMesh(app.mesh_defaults, DEFAULT_SCALE) : null);
}

/**
 * Panel count expected for `p`, from the current mesh when the two differ only in their
 * sizes (same grading radii): N ∝ h^-1.25.
 */
export function estimatePanels(p: MeshParams, mesh: { params: MeshParams; stats: { n_triangles: number } } | null): number | null {
  if (!mesh) return null;
  const q = mesh.params;
  if (q.dist_min !== p.dist_min || q.dist_max !== p.dist_max || q.min_circle_points !== p.min_circle_points) return null;
  const r = Math.sqrt((q.h_min / p.h_min) * (q.h_max / p.h_max));
  return mesh.stats.n_triangles * r ** 1.25;
}
