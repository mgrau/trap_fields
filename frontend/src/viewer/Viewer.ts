// Imperative three.js viewer. Scene units are millimeters; the public API uses meters.

import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { TrackballControls } from 'three/examples/jsm/controls/TrackballControls.js';
import { CSS2DObject, CSS2DRenderer } from 'three/examples/jsm/renderers/CSS2DRenderer.js';
import type { SurfMesh, Tess } from '../binary';
import { diverging, viridis } from '../colors';

export type Vec3 = [number, number, number];
export type ViewDir = '+x' | '-x' | '+y' | '-y' | '+z' | '-z' | 'iso';
export type SnapKind = 'vertex' | 'edge' | 'surface';

export interface Hit {
  point: Vec3; // meters
  body: number;
  face: number; // CAD face tag, -1 for vertex/edge snaps
  snap: SnapKind;
}

export interface BodyStyle {
  color: string;
  visible: boolean;
  selected: boolean;
}

/** Camera pose, to switch between views (e.g. the whole model and the trap region). */
export interface CameraState {
  position: THREE.Vector3;
  target: THREE.Vector3;
  up: THREE.Vector3;
  orthoHalf: number;
  orthoZoom: number;
  near: number;
  far: number;
}

export interface MeasureDraw {
  a: Vec3;
  b: Vec3;
  label: string;
}

const S = 1e3; // meters -> scene units (mm)
const SNAP_VERTEX_PX = 12;
const SNAP_EDGE_PX = 7;

interface BodyObj {
  mesh: THREE.Mesh;
  mat: THREE.MeshStandardMaterial;
  edges: THREE.LineSegments;
  edgeMat: THREE.LineBasicMaterial;
  faceOfTri: Uint32Array;
  edgePos: Float32Array; // scene units
  points: Float32Array; // scene units
  visible: boolean;
}

function textSprite(text: string, color: string): THREE.Sprite {
  const c = document.createElement('canvas');
  c.width = c.height = 64;
  const g = c.getContext('2d')!;
  g.font = 'bold 44px system-ui, sans-serif';
  g.fillStyle = color;
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.fillText(text, 32, 34);
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: new THREE.CanvasTexture(c), depthTest: false }));
  s.scale.setScalar(0.55);
  return s;
}

function pointMarker(color: number, size: number): THREE.Points {
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.Float32BufferAttribute([0, 0, 0], 3));
  const m = new THREE.PointsMaterial({ color, size, sizeAttenuation: false, depthTest: false, transparent: true });
  const p = new THREE.Points(g, m);
  p.renderOrder = 10;
  p.frustumCulled = false;
  return p;
}

export class Viewer {
  onHover?: (h: Hit | null) => void;
  onClick?: (h: Hit | null, e: PointerEvent) => void;
  snapping = true;

  private el: HTMLElement;
  private renderer: THREE.WebGLRenderer;
  private css = new CSS2DRenderer();
  private scene = new THREE.Scene();
  private persp = new THREE.PerspectiveCamera(30, 1, 0.001, 1e6);
  private ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, -1e3, 1e3);
  private cam: THREE.PerspectiveCamera | THREE.OrthographicCamera = this.persp;
  private controls!: OrbitControls | TrackballControls;
  private rotateMode: 'orbit' | 'free' = 'orbit';
  private light = new THREE.DirectionalLight(0xffffff, 2.0);
  private lightTarget = new THREE.Object3D();
  private model = new THREE.Group();
  private bodies: BodyObj[] = [];
  private overlay: { group: THREE.Group; mesh: THREE.Mesh; triBody: Uint32Array; mats: THREE.Material[] } | null = null;
  private overlayVisible = false;
  private measure = new THREE.Group();
  private trapGroup = new THREE.Group();
  private sliceGroup = new THREE.Group();
  /** faces picked for removal (and the one under the pointer while picking) */
  private faceGroup = new THREE.Group();
  private facePicking = false;
  private faceSel: number[] = [];
  private faceHover = -1;
  private hoverMarker = pointMarker(0xd97706, 10);
  private axes = new THREE.AxesHelper(1);
  private clip = new THREE.Plane(new THREE.Vector3(-1, 0, 0), 0);
  private clipOn = false;
  /** box around the trap region: six planes, the model is drawn only inside all of them */
  private region: THREE.Plane[] = [];
  private regionBox: THREE.Box3 | null = null;
  private gizmoScene = new THREE.Scene();
  private gizmoCam = new THREE.OrthographicCamera(-1.6, 1.6, 1.6, -1.6, 0.1, 10);
  private raycaster = new THREE.Raycaster();
  private needsRender = true;
  private raf = 0;
  private ro: ResizeObserver;
  private sceneSize = 10;
  private down: { x: number; y: number } | null = null;
  private hoverPending: PointerEvent | null = null;
  private lastPick: [number, number] = [NaN, NaN];
  private tmp = new THREE.Vector3();

  constructor(el: HTMLElement) {
    this.el = el;
    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, logarithmicDepthBuffer: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.localClippingEnabled = true;
    el.appendChild(this.renderer.domElement);
    Object.assign(this.css.domElement.style, { position: 'absolute', inset: '0', pointerEvents: 'none' });
    el.appendChild(this.css.domElement);

    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x8a94a6, 1.4));
    this.lightTarget.position.set(0, 0, -1);
    this.light.position.set(0.4, 0.6, 0);
    this.light.target = this.lightTarget;
    this.attachLight();
    this.scene.add(this.model, this.measure, this.hoverMarker, this.axes, this.trapGroup, this.sliceGroup, this.faceGroup);
    this.hoverMarker.visible = false;
    this.persp.up.set(0, 0, 1);
    this.ortho.up.set(0, 0, 1);
    this.persp.position.set(60, -80, 50);
    this.ortho.position.copy(this.persp.position);
    this.makeControls(new THREE.Vector3());
    this.buildGizmo();

    const c = this.renderer.domElement;
    c.addEventListener('pointerdown', (e) => (this.down = { x: e.clientX, y: e.clientY }));
    c.addEventListener('pointerup', (e) => {
      if (this.down && Math.hypot(e.clientX - this.down.x, e.clientY - this.down.y) < 4 && e.button === 0) {
        this.onClick?.(this.hitTest(e.clientX, e.clientY), e);
      }
      this.down = null;
    });
    c.addEventListener('pointermove', (e) => {
      if (e.buttons) return;
      this.hoverPending = e;
    });
    c.addEventListener('pointerleave', () => {
      this.hoverPending = null;
      this.showHover(null);
      this.onHover?.(null);
    });

    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(el);
    this.resize();
    const loop = () => {
      this.raf = requestAnimationFrame(loop);
      if (this.controls instanceof TrackballControls) this.controls.update();
      if (this.hoverPending) {
        const e = this.hoverPending;
        this.hoverPending = null;
        // picking walks every triangle, vertex and edge: skip it while the pointer rests
        // (a camera move still re-picks, since the controls change the view)
        if (e.clientX !== this.lastPick[0] || e.clientY !== this.lastPick[1] || this.needsRender) {
          this.lastPick = [e.clientX, e.clientY];
          const h = this.hitTest(e.clientX, e.clientY);
          this.showHover(h);
          this.onHover?.(h);
        }
      }
      if (this.needsRender) this.draw();
    };
    loop();
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.ro.disconnect();
    this.controls.dispose();
    this.clearBodies();
    this.setSurfaceMesh(null, 'size', []);
    this.renderer.dispose();
    this.el.innerHTML = '';
  }

  render() {
    this.needsRender = true;
  }

  // ---------------------------------------------------------------- geometry

  private clearBodies() {
    for (const b of this.bodies) {
      b.mesh.geometry.dispose();
      b.mat.dispose();
      b.edges.geometry.dispose();
      b.edgeMat.dispose();
    }
    this.model.clear();
    this.bodies = [];
    // face tags belong to the old tessellation
    this.faceSel = [];
    this.faceHover = -1;
    Viewer.clearGroup(this.faceGroup);
  }

  setTess(t: Tess | null, nBodies: number) {
    this.clearBodies();
    if (!t) return this.render();
    const nt = t.triBody.length;
    for (let b = 0; b < nBodies; b++) {
      const tris: number[] = [];
      for (let i = 0; i < nt; i++) if (t.triBody[i] === b) tris.push(i);
      const remap = new Map<number, number>();
      const pos: number[] = [];
      const idx = new Uint32Array(tris.length * 3);
      const faceOfTri = new Uint32Array(tris.length);
      tris.forEach((ti, k) => {
        faceOfTri[k] = t.triFace[ti];
        for (let j = 0; j < 3; j++) {
          const v = t.triangles[3 * ti + j];
          let r = remap.get(v);
          if (r === undefined) {
            r = pos.length / 3;
            remap.set(v, r);
            pos.push(t.positions[3 * v] * S, t.positions[3 * v + 1] * S, t.positions[3 * v + 2] * S);
          }
          idx[3 * k + j] = r;
        }
      });
      const g = new THREE.BufferGeometry();
      g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
      g.setIndex(new THREE.BufferAttribute(idx, 1));
      g.computeVertexNormals();
      const mat = new THREE.MeshStandardMaterial({
        color: 0x999999,
        metalness: 0.15,
        roughness: 0.55,
        side: THREE.DoubleSide,
        polygonOffset: true,
        polygonOffsetFactor: 1,
        polygonOffsetUnits: 1,
      });
      const mesh = new THREE.Mesh(g, mat);
      mesh.userData.body = b;

      const ep: number[] = [];
      for (let i = 0; i < t.edgeBody.length; i++) {
        if (t.edgeBody[i] !== b) continue;
        for (let j = 0; j < 6; j++) ep.push(t.edges[6 * i + j] * S);
      }
      const edgePos = new Float32Array(ep);
      const eg = new THREE.BufferGeometry();
      eg.setAttribute('position', new THREE.BufferAttribute(edgePos, 3));
      const edgeMat = new THREE.LineBasicMaterial({ color: 0x0b0d10, transparent: true, opacity: 0.85 });
      const edges = new THREE.LineSegments(eg, edgeMat);

      const pp: number[] = [];
      for (let i = 0; i < t.pointBody.length; i++) {
        if (t.pointBody[i] === b) pp.push(t.points[3 * i] * S, t.points[3 * i + 1] * S, t.points[3 * i + 2] * S);
      }
      this.model.add(mesh, edges);
      this.bodies.push({ mesh, mat, edges, edgeMat, faceOfTri, edgePos, points: new Float32Array(pp), visible: true });
    }
    const box = new THREE.Box3().setFromObject(this.model);
    this.sceneSize = Math.max(box.getSize(new THREE.Vector3()).length(), 1e-3);
    this.axes.scale.setScalar(this.sceneSize * 0.12);
    this.applyClipping();
    this.render();
  }

  setBodyStyles(styles: BodyStyle[], xray: boolean, edges: boolean) {
    this.bodies.forEach((b, i) => {
      const s = styles[i];
      if (!s) return;
      b.visible = s.visible;
      b.mat.color.set(s.color);
      b.mat.emissive.set(s.selected ? s.color : 0x000000);
      b.mat.emissiveIntensity = s.selected ? 0.45 : 0;
      if (b.mat.transparent !== xray) {
        b.mat.transparent = xray;
        b.mat.needsUpdate = true; // the opaque/transparent shader variant is chosen at compile time
      }
      b.mat.opacity = xray ? 0.3 : 1;
      b.mat.depthWrite = !xray;
      b.mesh.visible = s.visible && !this.overlayVisible;
      b.edges.visible = s.visible && edges;
      b.edgeMat.color.set(s.selected ? 0x1d4ed8 : xray ? 0x475569 : 0x0b0d10);
    });
    if (this.faceSel.length || this.faceHover >= 0) this.buildFaceHighlight();
    this.render();
  }

  // ------------------------------------------------- feature (face) picking

  /** Picking faces for feature removal: highlight the face under the pointer, no snapping. */
  setFacePicking(on: boolean) {
    if (on === this.facePicking) return;
    this.facePicking = on;
    this.faceHover = -1;
    this.buildFaceHighlight();
  }

  /** Faces (CAD tags) to highlight as picked. */
  setFaceHighlight(faces: number[]) {
    this.faceSel = faces;
    this.buildFaceHighlight();
  }

  private buildFaceHighlight() {
    Viewer.clearGroup(this.faceGroup);
    const sel = new Set(this.faceSel);
    // the electrode palette uses every saturated hue: a glowing light yellow stands out
    // against all of them, and white marks the face under the pointer
    const layers: [Set<number>, number, number][] = [[sel, 0xfde047, 1]];
    if (this.faceHover >= 0 && !sel.has(this.faceHover)) layers.push([new Set([this.faceHover]), 0xffffff, 0.6]);
    for (const [faces, color, opacity] of layers) {
      if (!faces.size) continue;
      const pos: number[] = [];
      const nrm: number[] = [];
      for (const b of this.bodies) {
        if (!b.visible) continue;
        const g = b.mesh.geometry;
        const p = g.getAttribute('position');
        const n = g.getAttribute('normal');
        const idx = g.getIndex()!;
        for (let k = 0; k < b.faceOfTri.length; k++) {
          if (!faces.has(b.faceOfTri[k])) continue;
          for (let j = 0; j < 3; j++) {
            const v = idx.getX(3 * k + j);
            pos.push(p.getX(v), p.getY(v), p.getZ(v));
            nrm.push(n.getX(v), n.getY(v), n.getZ(v));
          }
        }
      }
      if (!pos.length) continue;
      // copies, not shared attributes: disposing a geometry frees its attributes' buffers
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
      geo.setAttribute('normal', new THREE.Float32BufferAttribute(nrm, 3));
      const mat = new THREE.MeshStandardMaterial({
        color,
        emissive: color,
        emissiveIntensity: 0.6,
        metalness: 0,
        roughness: 0.6,
        side: THREE.DoubleSide,
        transparent: opacity < 1,
        opacity,
        depthWrite: false,
        // in front of the body's own triangles at the same depth
        polygonOffset: true,
        polygonOffsetFactor: -1,
        polygonOffsetUnits: -1,
      });
      mat.clippingPlanes = [...(this.clipOn ? [this.clip] : []), ...this.region];
      const m = new THREE.Mesh(geo, mat);
      m.renderOrder = 2;
      this.faceGroup.add(m);
    }
    this.render();
  }

  // ------------------------------------------------------------ BEM mesh

  /**
   * Show the BEM mesh colored by panel size (log viridis), electrode, or a per-triangle
   * value (diverging map, symmetric about 0, saturating at ±clip).
   * Returns the panel-size range (m).
   */
  setSurfaceMesh(
    m: SurfMesh | null,
    color: 'size' | 'body' | 'sigma',
    bodyColors: string[],
    values?: { data: ArrayLike<number>; clip: number } | null,
  ): [number, number] | null {
    if (this.overlay) {
      this.overlay.group.traverse((o) => {
        if (o instanceof THREE.Mesh || o instanceof THREE.LineSegments) o.geometry.dispose();
      });
      this.overlay.mats.forEach((x) => x.dispose());
      this.scene.remove(this.overlay.group);
      this.overlay = null;
    }
    if (!m) {
      this.setOverlayVisible(false);
      return null;
    }
    const nt = m.triBody.length;
    const nn = m.nodes.length / 3;
    const size = new Float32Array(nt);
    let lo = Infinity;
    let hi = 0;
    const P = (i: number) => [m.nodes[3 * i], m.nodes[3 * i + 1], m.nodes[3 * i + 2]];
    for (let t = 0; t < nt; t++) {
      const [a, b, c] = [P(m.triangles[3 * t]), P(m.triangles[3 * t + 1]), P(m.triangles[3 * t + 2])];
      const d = (u: number[], v: number[]) => Math.hypot(u[0] - v[0], u[1] - v[1], u[2] - v[2]);
      size[t] = (d(a, b) + d(b, c) + d(c, a)) / 3;
      lo = Math.min(lo, size[t]);
      hi = Math.max(hi, size[t]);
    }
    const pos = new Float32Array(nt * 9);
    const col = new Float32Array(nt * 9);
    const bc = bodyColors.map((c) => new THREE.Color(c));
    const llo = Math.log(lo);
    const lspan = Math.max(Math.log(hi) - llo, 1e-9);
    for (let t = 0; t < nt; t++) {
      const rgb: [number, number, number] =
        color === 'sigma' && values
          ? diverging(values.data[t] / values.clip)
          : color === 'size' || color === 'sigma'
            ? viridis((Math.log(size[t]) - llo) / lspan)
            : ((bc[m.triBody[t]] ?? new THREE.Color(0x999999)).toArray() as [number, number, number]);
      for (let j = 0; j < 3; j++) {
        const v = m.triangles[3 * t + j];
        for (let k = 0; k < 3; k++) {
          pos[9 * t + 3 * j + k] = m.nodes[3 * v + k] * S;
          col[9 * t + 3 * j + k] = rgb[k];
        }
      }
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
    g.setAttribute('color', new THREE.BufferAttribute(col, 3));
    g.computeVertexNormals();
    const mat = new THREE.MeshStandardMaterial({
      vertexColors: true,
      side: THREE.DoubleSide,
      metalness: 0,
      roughness: 0.8,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    const mesh = new THREE.Mesh(g, mat);

    // unique edges for the wireframe
    const seen = new Set<number>();
    const ep: number[] = [];
    for (let t = 0; t < nt; t++) {
      for (let j = 0; j < 3; j++) {
        const a = m.triangles[3 * t + j];
        const b = m.triangles[3 * t + ((j + 1) % 3)];
        const key = a < b ? a * nn + b : b * nn + a;
        if (seen.has(key)) continue;
        seen.add(key);
        ep.push(m.nodes[3 * a] * S, m.nodes[3 * a + 1] * S, m.nodes[3 * a + 2] * S);
        ep.push(m.nodes[3 * b] * S, m.nodes[3 * b + 1] * S, m.nodes[3 * b + 2] * S);
      }
    }
    const eg = new THREE.BufferGeometry();
    eg.setAttribute('position', new THREE.Float32BufferAttribute(ep, 3));
    const emat = new THREE.LineBasicMaterial({ color: 0x0b0d10, transparent: true, opacity: 0.55 });
    const group = new THREE.Group();
    group.add(mesh, new THREE.LineSegments(eg, emat));
    group.visible = this.overlayVisible;
    this.scene.add(group);
    this.overlay = { group, mesh, triBody: m.triBody, mats: [mat, emat] };
    this.applyClipping();
    this.render();
    return [lo, hi];
  }

  setOverlayVisible(v: boolean) {
    this.overlayVisible = v && !!this.overlay;
    if (this.overlay) this.overlay.group.visible = this.overlayVisible;
    this.faceGroup.visible = !this.overlayVisible;
    for (const b of this.bodies) b.mesh.visible = b.visible && !this.overlayVisible;
    this.render();
  }

  // ------------------------------------------------------------ overlays

  private static clearGroup(g: THREE.Group) {
    g.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.Line || o instanceof THREE.Points) {
        o.geometry.dispose();
        const m = o.material as THREE.Material & { map?: THREE.Texture | null };
        m.map?.dispose();
        m.dispose();
      }
    });
    g.clear();
  }

  private ionGroup = new THREE.Group();

  /** Ion positions of a chain (m). */
  setIons(ions: Vec3[] | null) {
    Viewer.clearGroup(this.ionGroup);
    if (!this.ionGroup.parent) this.scene.add(this.ionGroup);
    for (const q of ions ?? []) {
      const p = pointMarker(0xdb2777, 9);
      p.position.set(q[0] * S, q[1] * S, q[2] * S);
      this.ionGroup.add(p);
    }
    this.render();
  }

  /** Ion equilibrium and principal axes (each drawn ±len, m), and the escape route over the barrier. */
  setTrapMarkers(m: { eq: Vec3; rfNull: Vec3; axes: { dir: Vec3; color: string }[]; len: number; escape?: { path: Vec3[]; barrier: Vec3 } } | null) {
    Viewer.clearGroup(this.trapGroup);
    if (m) {
      const eq = new THREE.Vector3(...m.eq).multiplyScalar(S);
      for (const a of m.axes) {
        const d = new THREE.Vector3(...a.dir).multiplyScalar(m.len * S);
        const line = new THREE.Line(
          new THREE.BufferGeometry().setFromPoints([eq.clone().sub(d), eq.clone().add(d)]),
          new THREE.LineBasicMaterial({ color: a.color, depthTest: false, transparent: true }),
        );
        line.renderOrder = 11;
        this.trapGroup.add(line);
      }
      const ion = pointMarker(0xdb2777, 11);
      ion.position.copy(eq);
      const nul = pointMarker(0x0f172a, 6);
      nul.position.set(...m.rfNull).multiplyScalar(S);
      this.trapGroup.add(ion, nul);
      if (m.escape && m.escape.path.length > 1) {
        const route = new THREE.Line(
          new THREE.BufferGeometry().setFromPoints(m.escape.path.map((p) => new THREE.Vector3(...p).multiplyScalar(S))),
          new THREE.LineDashedMaterial({ color: 0xea580c, dashSize: 0.5 * m.len * S, gapSize: 0.25 * m.len * S, depthTest: false, transparent: true }),
        );
        route.computeLineDistances();
        route.renderOrder = 11;
        const top = pointMarker(0xea580c, 8);
        top.position.set(...m.escape.barrier).multiplyScalar(S);
        this.trapGroup.add(route, top);
      }
    }
    this.render();
  }

  /** Colored plane (RGBA texture, n×n, rows along v) through `center` spanning ±half along the u and v axes. */
  setSlice(sl: { center: Vec3; uAxis: number; vAxis: number; half: number; n: number; rgba: Uint8Array; size: number } | null) {
    Viewer.clearGroup(this.sliceGroup);
    if (sl) {
      // values sit on grid nodes from −half to +half: the texels span half a step more
      const ext = sl.n > 1 ? (sl.half * sl.n) / (sl.n - 1) : sl.half;
      const e = (k: number) => {
        const v = new THREE.Vector3();
        v.setComponent(k, ext * S);
        return v;
      };
      const c = new THREE.Vector3(...sl.center).multiplyScalar(S);
      const [u, v] = [e(sl.uAxis), e(sl.vAxis)];
      const corners = [c.clone().sub(u).sub(v), c.clone().add(u).sub(v), c.clone().add(u).add(v), c.clone().sub(u).add(v)];
      const g = new THREE.BufferGeometry().setFromPoints(corners);
      g.setIndex([0, 1, 2, 0, 2, 3]);
      g.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, 1, 1, 0, 1], 2));
      // the image is already resampled finer than the grid (sliceImage): filter it smoothly,
      // with mipmaps so the contour lines do not shimmer when the plane is seen small
      const tex = new THREE.DataTexture(sl.rgba, sl.size, sl.size, THREE.RGBAFormat);
      tex.colorSpace = THREE.SRGBColorSpace;
      tex.magFilter = THREE.LinearFilter;
      tex.minFilter = THREE.LinearMipmapLinearFilter;
      tex.generateMipmaps = true;
      tex.needsUpdate = true;
      const mat = new THREE.MeshBasicMaterial({ map: tex, side: THREE.DoubleSide, transparent: true, depthWrite: false });
      const mesh = new THREE.Mesh(g, mat);
      mesh.renderOrder = 5;
      this.sliceGroup.add(mesh);
    }
    this.render();
  }

  setMeasurements(items: MeasureDraw[], pending: Vec3 | null) {
    this.measure.traverse((o) => {
      if (o instanceof THREE.Line || o instanceof THREE.Points) {
        o.geometry.dispose();
        (o.material as THREE.Material).dispose();
      }
    });
    this.measure.clear();
    const lineMat = new THREE.LineBasicMaterial({ color: 0xd97706, depthTest: false, transparent: true });
    for (const m of items) {
      const a = new THREE.Vector3(...m.a).multiplyScalar(S);
      const b = new THREE.Vector3(...m.b).multiplyScalar(S);
      const line = new THREE.Line(new THREE.BufferGeometry().setFromPoints([a, b]), lineMat.clone());
      line.renderOrder = 9;
      const ends = new THREE.Points(
        new THREE.BufferGeometry().setFromPoints([a, b]),
        new THREE.PointsMaterial({ color: 0xd97706, size: 8, sizeAttenuation: false, depthTest: false, transparent: true }),
      );
      ends.renderOrder = 10;
      const div = document.createElement('div');
      div.className = 'measure-label';
      div.textContent = m.label;
      const label = new CSS2DObject(div);
      label.position.copy(a).lerp(b, 0.5);
      this.measure.add(line, ends, label);
    }
    lineMat.dispose();
    if (pending) {
      const p = pointMarker(0xdc2626, 10);
      p.position.set(...pending).multiplyScalar(S);
      this.measure.add(p);
    }
    this.render();
  }

  private showHover(h: Hit | null) {
    if (this.facePicking) {
      const f = h && h.snap === 'surface' ? h.face : -1;
      if (f !== this.faceHover) {
        this.faceHover = f;
        this.buildFaceHighlight();
      }
    }
    this.hoverMarker.visible = !!h && h.snap !== 'surface';
    if (h) {
      this.hoverMarker.position.set(h.point[0] * S, h.point[1] * S, h.point[2] * S);
      (this.hoverMarker.material as THREE.PointsMaterial).color.set(h.snap === 'vertex' ? 0x16a34a : 0xd97706);
    }
    this.render();
  }

  setAxesVisible(v: boolean) {
    this.axes.visible = v;
    this.render();
  }

  /** Section plane perpendicular to `axis` at `offset` (m); keeps the side below unless flipped. */
  setSection(axis: 'x' | 'y' | 'z' | null, offset: number, flip: boolean) {
    this.clipOn = axis !== null;
    if (axis) {
      const n = new THREE.Vector3(axis === 'x' ? 1 : 0, axis === 'y' ? 1 : 0, axis === 'z' ? 1 : 0);
      const sgn = flip ? 1 : -1;
      this.clip.set(n.multiplyScalar(sgn), -sgn * offset * S);
    }
    this.applyClipping();
    this.render();
  }

  /** Clip the model to an axis-aligned box (center and half-size in m); null removes it. */
  setRegion(r: { center: Vec3; half: number } | null) {
    if (r) {
      const c = new THREE.Vector3(...r.center).multiplyScalar(S);
      const h = r.half * S;
      this.regionBox = new THREE.Box3(c.clone().subScalar(h), c.clone().addScalar(h));
      this.region = [0, 1, 2].flatMap((k) => {
        const n = new THREE.Vector3();
        n.setComponent(k, 1);
        // keep c_k − h ≤ x_k ≤ c_k + h
        return [new THREE.Plane(n.clone(), -(c.getComponent(k) - h)), new THREE.Plane(n.clone().negate(), c.getComponent(k) + h)];
      });
    } else {
      this.regionBox = null;
      this.region = [];
    }
    this.applyClipping();
    this.render();
  }

  /** Whether a point (scene units) is on the drawn side of the section plane and inside the region. */
  private shown(p: THREE.Vector3) {
    if (this.clipOn && this.clip.distanceToPoint(p) < 0) return false;
    return this.region.every((pl) => pl.distanceToPoint(p) >= -1e-9);
  }

  private applyClipping() {
    const planes = [...(this.clipOn ? [this.clip] : []), ...this.region];
    const mats: THREE.Material[] = this.bodies.flatMap((b) => [b.mat, b.edgeMat]);
    this.faceGroup.traverse((o) => {
      if (o instanceof THREE.Mesh) mats.push(o.material as THREE.Material);
    });
    if (this.overlay) mats.push(...this.overlay.mats);
    for (const m of mats) {
      m.clippingPlanes = planes;
      m.needsUpdate = true;
    }
  }

  // -------------------------------------------------------------- camera

  private attachLight() {
    this.light.removeFromParent();
    this.lightTarget.removeFromParent();
    this.cam.add(this.light, this.lightTarget);
    this.scene.remove(this.persp, this.ortho);
    this.scene.add(this.cam);
  }

  private makeControls(target: THREE.Vector3) {
    this.controls?.dispose();
    const dom = this.renderer.domElement;
    if (this.rotateMode === 'orbit') {
      const c = new OrbitControls(this.cam, dom);
      c.screenSpacePanning = true;
      c.zoomToCursor = true;
      c.target.copy(target);
      c.addEventListener('change', () => this.render());
      c.update();
      this.controls = c;
    } else {
      const c = new TrackballControls(this.cam, dom);
      c.rotateSpeed = 3;
      c.staticMoving = true;
      c.target.copy(target);
      c.addEventListener('change', () => this.render());
      c.handleResize();
      this.controls = c;
    }
  }

  setRotateMode(mode: 'orbit' | 'free') {
    if (mode === this.rotateMode) return;
    this.rotateMode = mode;
    if (mode === 'orbit') {
      // re-establish an upright camera
      this.cam.up.set(0, 0, 1);
      this.cam.lookAt(this.controls.target);
    }
    this.makeControls(this.controls.target.clone());
    this.render();
  }

  setProjection(p: 'persp' | 'ortho') {
    const next = p === 'persp' ? this.persp : this.ortho;
    if (next === this.cam) return;
    const target = this.controls.target.clone();
    const dist = this.cam.position.distanceTo(target);
    next.position.copy(this.cam.position);
    next.quaternion.copy(this.cam.quaternion);
    next.up.copy(this.cam.up);
    if (next === this.ortho) {
      this.ortho.zoom = 1;
      this.setOrthoHalfHeight(dist * Math.tan(THREE.MathUtils.degToRad(this.persp.fov / 2)));
    } else {
      // keep apparent size when going back to perspective
      const hh = (this.ortho.top - this.ortho.bottom) / 2 / this.ortho.zoom;
      const d = hh / Math.tan(THREE.MathUtils.degToRad(this.persp.fov / 2));
      const dir = new THREE.Vector3().subVectors(this.ortho.position, target).normalize();
      next.position.copy(target).addScaledVector(dir, d);
    }
    this.cam = next;
    this.attachLight();
    this.makeControls(target);
    this.resize();
  }

  private setOrthoHalfHeight(hh: number) {
    const { clientWidth: w, clientHeight: h } = this.el;
    const aspect = w / Math.max(h, 1);
    Object.assign(this.ortho, { left: -hh * aspect, right: hh * aspect, top: hh, bottom: -hh });
    this.ortho.updateProjectionMatrix();
  }

  private visibleBox(): THREE.Box3 {
    const box = new THREE.Box3();
    for (const b of this.bodies) if (b.visible) box.expandByObject(b.mesh);
    if (box.isEmpty()) box.setFromCenterAndSize(new THREE.Vector3(), new THREE.Vector3(10, 10, 10));
    if (this.regionBox) {
      const inside = box.clone().intersect(this.regionBox);
      return inside.isEmpty() ? this.regionBox.clone() : inside;
    }
    return box;
  }

  fit() {
    const dir = new THREE.Vector3().subVectors(this.cam.position, this.controls.target).normalize();
    this.frame(dir.lengthSq() > 0 ? dir : new THREE.Vector3(1, -1, 1).normalize(), this.cam.up.clone());
  }

  view(d: ViewDir) {
    const dirs: Record<ViewDir, [number, number, number]> = {
      '+x': [1, 0, 0],
      '-x': [-1, 0, 0],
      '+y': [0, 1, 0],
      '-y': [0, -1, 0],
      '+z': [0, 0, 1],
      '-z': [0, 0, -1],
      iso: [1, -1, 0.8],
    };
    const up = d === '+z' || d === '-z' ? new THREE.Vector3(0, 1, 0) : new THREE.Vector3(0, 0, 1);
    this.frame(new THREE.Vector3(...dirs[d]).normalize(), up);
  }

  /** Frame a sphere (center and radius in m), from `from` (toward the camera) or keeping the viewing direction. */
  focus(center: Vec3, radius: number, from?: { dir: Vec3; up: Vec3 }) {
    const keep = new THREE.Vector3().subVectors(this.cam.position, this.controls.target).normalize();
    const dir = from ? new THREE.Vector3(...from.dir).normalize() : keep.lengthSq() > 0 ? keep : new THREE.Vector3(1, -1, 0.8).normalize();
    const up = from ? new THREE.Vector3(...from.up) : this.cam.up.clone();
    const sphere = new THREE.Sphere(new THREE.Vector3(...center).multiplyScalar(S), radius * S);
    this.frame(dir, up, sphere);
  }

  getCamera(): CameraState {
    return {
      position: this.cam.position.clone(),
      target: this.controls.target.clone(),
      up: this.cam.up.clone(),
      orthoHalf: (this.ortho.top - this.ortho.bottom) / 2,
      orthoZoom: this.ortho.zoom,
      near: this.persp.near,
      far: this.persp.far,
    };
  }

  setCamera(c: CameraState) {
    for (const cam of [this.persp, this.ortho]) {
      cam.position.copy(c.position);
      cam.up.copy(c.up);
      cam.lookAt(c.target);
    }
    Object.assign(this.persp, { near: c.near, far: c.far });
    this.persp.updateProjectionMatrix();
    const d = c.position.distanceTo(c.target);
    const r = c.near * 1e4;
    this.ortho.near = -10 * r;
    this.ortho.far = d + 10 * r;
    this.ortho.zoom = c.orthoZoom;
    this.setOrthoHalfHeight(c.orthoHalf);
    this.makeControls(c.target);
    this.render();
  }

  private frame(dir: THREE.Vector3, up: THREE.Vector3, target?: THREE.Sphere) {
    const sphere = target ?? this.visibleBox().getBoundingSphere(new THREE.Sphere());
    const r = Math.max(sphere.radius, 1e-3) * 1.08;
    const d = r / Math.sin(THREE.MathUtils.degToRad(this.persp.fov / 2));
    this.cam.up.copy(up);
    this.cam.position.copy(sphere.center).addScaledVector(dir, d);
    this.cam.lookAt(sphere.center);
    this.ortho.near = -10 * r;
    this.ortho.far = d + 10 * r;
    if (this.cam === this.ortho) {
      this.ortho.zoom = 1;
      this.setOrthoHalfHeight(r);
    }
    this.persp.near = r * 1e-4;
    this.persp.far = r * 1e3;
    this.persp.updateProjectionMatrix();
    this.makeControls(sphere.center); // OrbitControls captures `up` at construction
    this.render();
  }

  private resize() {
    const { clientWidth: w, clientHeight: h } = this.el;
    if (!w || !h) return;
    this.renderer.setSize(w, h);
    this.css.setSize(w, h);
    this.persp.aspect = w / h;
    this.persp.updateProjectionMatrix();
    this.setOrthoHalfHeight((this.ortho.top - this.ortho.bottom) / 2 || 10);
    if (this.controls instanceof TrackballControls) this.controls.handleResize();
    this.render();
  }

  // --------------------------------------------------------------- gizmo

  private buildGizmo() {
    const axes: [THREE.Vector3, number, string][] = [
      [new THREE.Vector3(1, 0, 0), 0xdc2626, 'X'],
      [new THREE.Vector3(0, 1, 0), 0x16a34a, 'Y'],
      [new THREE.Vector3(0, 0, 1), 0x2563eb, 'Z'],
    ];
    for (const [d, c, name] of axes) {
      this.gizmoScene.add(new THREE.ArrowHelper(d, new THREE.Vector3(), 1, c, 0.28, 0.16));
      const s = textSprite(name, `#${c.toString(16).padStart(6, '0')}`);
      s.position.copy(d).multiplyScalar(1.32);
      this.gizmoScene.add(s);
    }
  }

  private draw() {
    this.needsRender = false;
    const r = this.renderer;
    const { clientWidth: w, clientHeight: h } = this.el;
    r.setScissorTest(false);
    r.setViewport(0, 0, w, h);
    r.clear();
    r.render(this.scene, this.cam);
    this.css.render(this.scene, this.cam);
    // orientation gizmo, lower-left
    const g = 96;
    this.gizmoCam.position.set(0, 0, 4).applyQuaternion(this.cam.quaternion);
    this.gizmoCam.quaternion.copy(this.cam.quaternion);
    r.autoClear = false;
    r.setScissorTest(true);
    r.setScissor(8, 8, g, g);
    r.setViewport(8, 8, g, g);
    r.clearDepth();
    r.render(this.gizmoScene, this.gizmoCam);
    r.setScissorTest(false);
    r.autoClear = true;
  }

  // ------------------------------------------------------------- picking

  private toScreen(v: THREE.Vector3, rect: DOMRect): [number, number, boolean] {
    const p = this.tmp.copy(v).project(this.cam);
    return [((p.x + 1) / 2) * rect.width, ((1 - p.y) / 2) * rect.height, p.z > -1 && p.z < 1];
  }

  private hitTest(cx: number, cy: number): Hit | null {
    const rect = this.renderer.domElement.getBoundingClientRect();
    const sx = cx - rect.left;
    const sy = cy - rect.top;
    const ndc = new THREE.Vector2((sx / rect.width) * 2 - 1, -(sy / rect.height) * 2 + 1);
    this.raycaster.setFromCamera(ndc, this.cam);
    const ray = this.raycaster.ray;

    const targets: THREE.Object3D[] = this.overlayVisible && this.overlay ? [this.overlay.mesh] : this.bodies.filter((b) => b.visible).map((b) => b.mesh);
    const hits = this.raycaster.intersectObjects(targets, false).filter((h) => this.shown(h.point));
    let surf: Hit | null = null;
    let surfDist = Infinity;
    if (hits.length) {
      const h = hits[0];
      surfDist = h.distance;
      let body = -1;
      let face = -1;
      if (this.overlay && h.object === this.overlay.mesh) {
        body = this.overlay.triBody[h.faceIndex ?? 0] ?? -1;
      } else {
        body = h.object.userData.body as number;
        face = this.bodies[body].faceOfTri[h.faceIndex ?? 0];
      }
      surf = { point: [h.point.x / S, h.point.y / S, h.point.z / S], body, face, snap: 'surface' };
    }
    // picking faces: the face under the pointer, not a vertex or edge near it
    if (!this.snapping || this.facePicking) return surf;

    const tol = this.sceneSize * 2e-3;
    const visibleAt = (v: THREE.Vector3) => {
      if (!this.shown(v)) return false;
      const along = v.clone().sub(ray.origin).dot(ray.direction);
      return along <= surfDist + tol;
    };
    // vertex snap
    let best: Hit | null = null;
    let bestPx = SNAP_VERTEX_PX;
    const v = new THREE.Vector3();
    this.bodies.forEach((b, bi) => {
      if (!b.visible) return;
      for (let i = 0; i < b.points.length; i += 3) {
        v.set(b.points[i], b.points[i + 1], b.points[i + 2]);
        const [px, py, inside] = this.toScreen(v, rect);
        const d = Math.hypot(px - sx, py - sy);
        if (inside && d < bestPx && visibleAt(v)) {
          bestPx = d;
          best = { point: [v.x / S, v.y / S, v.z / S], body: bi, face: -1, snap: 'vertex' };
        }
      }
    });
    if (best) return best;
    // edge snap
    bestPx = SNAP_EDGE_PX;
    const a = new THREE.Vector3();
    const c = new THREE.Vector3();
    this.bodies.forEach((b, bi) => {
      if (!b.visible || !b.edges.visible) return;
      const e = b.edgePos;
      for (let i = 0; i < e.length; i += 6) {
        a.set(e[i], e[i + 1], e[i + 2]);
        c.set(e[i + 3], e[i + 4], e[i + 5]);
        const [ax, ay, ia] = this.toScreen(a, rect);
        const [bx, by, ib] = this.toScreen(c, rect);
        if (!ia || !ib) continue;
        const dx = bx - ax;
        const dy = by - ay;
        const L2 = dx * dx + dy * dy;
        const t = L2 > 0 ? Math.min(1, Math.max(0, ((sx - ax) * dx + (sy - ay) * dy) / L2)) : 0;
        const d = Math.hypot(ax + t * dx - sx, ay + t * dy - sy);
        if (d < bestPx) {
          const p = a.clone().lerp(c, t);
          if (!visibleAt(p)) continue;
          bestPx = d;
          best = { point: [p.x / S, p.y / S, p.z / S], body: bi, face: -1, snap: 'edge' };
        }
      }
    });
    return best ?? surf;
  }
}
