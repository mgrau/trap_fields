// Decoders for the backend's binary geometry buffers (little-endian, 4-byte aligned).

export interface Tess {
  positions: Float32Array; // xyz per vertex, meters
  triangles: Uint32Array;
  triBody: Uint32Array;
  triFace: Uint32Array;
  edges: Float32Array; // segment endpoints (2 x xyz)
  edgeBody: Uint32Array;
  points: Float32Array; // CAD vertices
  pointBody: Uint32Array;
}

export interface SurfMesh {
  nodes: Float32Array;
  triangles: Uint32Array;
  triBody: Uint32Array;
  triFace: Uint32Array;
}

function reader(buf: ArrayBuffer, magic: string, nHeader: number) {
  const m = new TextDecoder().decode(new Uint8Array(buf, 0, 4));
  if (m !== magic) throw new Error(`bad buffer magic ${m}, expected ${magic}`);
  const header = new Uint32Array(buf, 4, nHeader);
  let off = 4 + 4 * nHeader;
  const take = <T extends Float32Array | Uint32Array>(C: new (b: ArrayBuffer, o: number, n: number) => T, n: number): T => {
    const a = new C(buf, off, n);
    off += 4 * n;
    return a;
  };
  return { header, take };
}

export function decodeTess(buf: ArrayBuffer): Tess {
  const { header, take } = reader(buf, 'TFT1', 4);
  const [nv, nt, ne, np] = header;
  return {
    positions: take(Float32Array, 3 * nv),
    triangles: take(Uint32Array, 3 * nt),
    triBody: take(Uint32Array, nt),
    triFace: take(Uint32Array, nt),
    edges: take(Float32Array, 6 * ne),
    edgeBody: take(Uint32Array, ne),
    points: take(Float32Array, 3 * np),
    pointBody: take(Uint32Array, np),
  };
}

export function decodeMesh(buf: ArrayBuffer): SurfMesh {
  const { header, take } = reader(buf, 'TFM1', 2);
  const [nn, nt] = header;
  return {
    nodes: take(Float32Array, 3 * nn),
    triangles: take(Uint32Array, 3 * nt),
    triBody: take(Uint32Array, nt),
    triFace: take(Uint32Array, nt),
  };
}

export interface Sigma {
  nBasis: number;
  nTri: number;
  /** surface charge density (C/m²) of basis b at triangle t: data[b * nTri + t] */
  data: Float64Array;
}

export function decodeSigma(buf: ArrayBuffer): Sigma {
  const { header } = reader(buf, 'TFS1', 3);
  const [nBasis, nTri] = header;
  return { nBasis, nTri, data: new Float64Array(buf, 16, nBasis * nTri) };
}
