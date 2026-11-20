// Faces picked for feature removal (Geometry → Remove features). Kept apart from the main
// store: only the panel and the viewer's highlight use it.

import { create } from 'zustand';
import { api, type FeatureFace } from './api';
import { useStore } from './store';
import type { Hit } from './viewer/Viewer';

interface FeaturePick {
  /** body the picked faces belong to (one removal acts on one body) */
  body: number | null;
  faces: FeatureFace[];
  /** a click selects the whole feature around the face, not just the face */
  grow: boolean;
}

export const useFeatures = create<FeaturePick>(() => ({ body: null, faces: [], grow: true }));

export const clearFeaturePick = () => useFeatures.setState({ body: null, faces: [] });

// face tags belong to one tessellation: a new model (or a removal) invalidates the pick
useStore.subscribe((s, p) => {
  if (s.tess !== p.tess) clearFeaturePick();
});

/**
 * Click in the viewer while picking. A plain click selects the face (grown to its feature
 * when `grow` is on); shift/ctrl/cmd-click adds or removes single faces of the same body.
 */
export async function pickFeatureFace(h: Hit | null, toggle: boolean) {
  const { faces, body, grow } = useFeatures.getState();
  if (!h || h.face < 0) {
    if (!toggle) clearFeaturePick();
    return;
  }
  if (toggle && faces.some((f) => f.tag === h.face)) {
    const rest = faces.filter((f) => f.tag !== h.face);
    useFeatures.setState({ faces: rest, body: rest.length ? body : null });
    return;
  }
  try {
    const r = await api.featureFaces(h.face, grow && !toggle);
    if (toggle && body === r.body) {
      const have = new Set(faces.map((f) => f.tag));
      useFeatures.setState({ faces: [...faces, ...r.faces.filter((f) => !have.has(f.tag))] });
    } else useFeatures.setState({ body: r.body, faces: r.faces });
  } catch (e) {
    useStore.getState().set({ error: (e as Error).message });
  }
}
