// Imperative access to the 3D view (camera commands) and the trap-region box, shared by
// the viewer, its toolbar and the panels.
import type { Vec3 } from '../api';
import type { useStore } from '../store';
import type { Viewer, ViewDir } from './Viewer';

let viewerRef: Viewer | null = null;

export function setViewer(v: Viewer | null) {
  viewerRef = v;
}

export const viewerCommands = {
  view: (d: ViewDir) => viewerRef?.view(d),
  fit: () => viewerRef?.fit(),
  /** frame a sphere (m) */
  focus: (center: Vec3, radius: number) => viewerRef?.focus(center, radius),
};

/** Color-scale span (decades) of the charge-density display. */
export const SIGMA_DECADES = 3;

/** The trap region: a box around the expansion center (else the mesh refinement center). */
export function trapRegion(st: ReturnType<typeof useStore.getState>): { center: Vec3; half: number } {
  const center = st.solution?.center ?? st.app?.project.mesh_params?.center ?? [0, 0, 0];
  const near = st.solution?.r_min ?? st.app?.project.mesh_params?.dist_min ?? st.app?.mesh_defaults?.dist_min ?? 1e-3;
  return { center, half: st.view.regionHalf ?? 2 * near };
}
