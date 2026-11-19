export type Units = 'mm' | 'um';

export const unitScale = (u: Units) => (u === 'mm' ? 1e3 : 1e6);
export const unitLabel = (u: Units) => (u === 'mm' ? 'mm' : 'µm');
const digits = (u: Units) => (u === 'mm' ? 4 : 2);

/** meters -> number in display units (as string, fixed precision) */
export function num(m: number, u: Units, d = digits(u)): string {
  const v = m * unitScale(u);
  return (Math.abs(v) < 0.5 * 10 ** -d ? 0 : v).toFixed(d);
}

/** meters -> "1.2345 mm" */
export function len(m: number, u: Units, d?: number): string {
  return `${num(m, u, d)} ${unitLabel(u)}`;
}

export function vec(p: ArrayLike<number>, u: Units, d?: number): string {
  return `(${num(p[0], u, d)}, ${num(p[1], u, d)}, ${num(p[2], u, d)}) ${unitLabel(u)}`;
}

/** display units -> meters */
export const toMeters = (v: number, u: Units) => v / unitScale(u);

export const fmtVolume = (m3: number) => `${(m3 * 1e9).toFixed(3)} mm³`;
export const fmtArea = (m2: number) => `${(m2 * 1e6).toFixed(3)} mm²`;
