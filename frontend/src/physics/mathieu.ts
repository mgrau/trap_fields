// Mathieu equation u'' + (a − 2q cos 2τ) u = 0 (τ = Ωt/2), as used for the trap's a, q:
// characteristic values bounding the stability regions, and the characteristic exponent
// β (secular frequency ω = βΩ/2).
//
// The characteristic values are eigenvalues of symmetric tridiagonal matrices in the
// Fourier basis of the periodic solutions (DLMF 28.6, Abramowitz & Stegun 20.2):
//   a_2n   : cos 2kτ      diag (2k)², off-diagonal q (√2 q between k = 0 and 1)
//   a_2n+1 : cos (2k+1)τ  diag (2k+1)², first entry 1 + q
//   b_2n+1 : sin (2k+1)τ  diag (2k+1)², first entry 1 − q
//   b_2n+2 : sin (2k+2)τ  diag (2k+2)²
// For q ≥ 0, a_0 < b_1 < a_1 < b_2 < a_2 < …; u is stable for a_n < a < b_{n+1}.
// Iso-β curves in the first region are the lowest eigenvalue of the Floquet matrix:
// diag (β + 2k)², k = −K…K, off-diagonal q.

const K = 24;

/** k-th smallest eigenvalue (0-based) of the symmetric tridiagonal matrix (d, e). */
function tridiagEigen(d: number[], e: number[], k: number): number {
  const n = d.length;
  // Sturm count: eigenvalues below x
  const below = (x: number) => {
    let c = 0;
    let q = d[0] - x;
    if (q < 0) c++;
    for (let i = 1; i < n; i++) {
      q = d[i] - x - (e[i - 1] * e[i - 1]) / (q === 0 ? 1e-300 : q);
      if (q < 0) c++;
    }
    return c;
  };
  // Gershgorin bounds
  let lo = Infinity;
  let hi = -Infinity;
  for (let i = 0; i < n; i++) {
    const r = Math.abs(i > 0 ? e[i - 1] : 0) + Math.abs(i < n - 1 ? e[i] : 0);
    lo = Math.min(lo, d[i] - r);
    hi = Math.max(hi, d[i] + r);
  }
  for (let it = 0; it < 200 && hi - lo > 1e-13 * Math.max(1, Math.abs(lo) + Math.abs(hi)); it++) {
    const mid = 0.5 * (lo + hi);
    if (below(mid) > k) hi = mid;
    else lo = mid;
  }
  return 0.5 * (lo + hi);
}

/** Characteristic value a_r(q) ('a') or b_r(q) ('b'), q ≥ 0. */
export function charValue(kind: 'a' | 'b', r: number, q: number): number {
  q = Math.abs(q);
  const even = r % 2 === 0;
  const n = Math.floor(r / 2);
  const d: number[] = [];
  const e: number[] = new Array(K - 1).fill(q);
  if (kind === 'a' && even) {
    for (let k = 0; k < K; k++) d.push((2 * k) ** 2);
    e[0] = Math.SQRT2 * q;
    return tridiagEigen(d, e, n);
  }
  if (kind === 'a') {
    for (let k = 0; k < K; k++) d.push((2 * k + 1) ** 2);
    d[0] += q;
    return tridiagEigen(d, e, n);
  }
  if (r < 1) throw new Error('b_0 does not exist');
  if (!even) {
    for (let k = 0; k < K; k++) d.push((2 * k + 1) ** 2);
    d[0] -= q;
    return tridiagEigen(d, e, n);
  }
  for (let k = 0; k < K; k++) d.push((2 * k + 2) ** 2);
  return tridiagEigen(d, e, n - 1);
}

/** a on the iso-β curve of the first stability region (0 ≤ β ≤ 1) at q. */
export function isoBetaA(beta: number, q: number): number {
  const d: number[] = [];
  for (let k = -K; k <= K; k++) d.push((beta + 2 * k) ** 2);
  return tridiagEigen(d, new Array(2 * K).fill(Math.abs(q)), 0);
}

/** Which stability region (1, 2, …) contains (a, q), or 0 if unstable. */
export function region(a: number, q: number): number {
  for (let r = 0; r < 6; r++) {
    const lo = charValue('a', r, q);
    const hi = charValue('b', r + 1, q);
    if (a > lo && a < hi) return r + 1;
    if (a < lo) return 0;
  }
  return 0;
}

/** Characteristic exponent β of a point in the first stability region, else null. */
export function betaOf(a: number, q: number): number | null {
  if (region(a, q) !== 1) return null;
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 60; i++) {
    const mid = 0.5 * (lo + hi);
    if (isoBetaA(mid, q) < a) lo = mid;
    else hi = mid;
  }
  return 0.5 * (lo + hi);
}
