/** The example projects the app is shipped with (served as bundles, see examples-plugin.ts). */
export const EXAMPLES = [
  {
    name: 'paul_trap',
    title: 'Stretched 3D Paul trap (March 1997)',
    about: 'The quadrupole ion trap of R. E. March, J. Mass Spectrom. 32, 351 (1997): hyperboloid ring r0 = 1.00 cm, endcaps at z0 = 0.783 cm; 757 V at 1.05 MHz, m/z 134.',
  },
  {
    name: 'surface_trap',
    title: 'Five-wire surface trap (Daniilidis 2011)',
    about: 'Daniilidis et al., New J. Phys. 13, 013032 (2011): RF rails 400 and 200 µm, 250 µm center, 10 pairs of DC segments, ion ≈ 240 µm above the chip. ⁴⁰Ca⁺, 100 V at 15 MHz.',
  },
  {
    name: 'segmented_trap',
    title: 'Segmented four-rod trap (Miao 2022)',
    about: 'Miao et al., arXiv:2201.03377 (2022): rods 14.2 mm in diameter, r0 = 6.2 mm, segmented 20/40/20 mm. ⁴⁰Ca⁺, 150 V at 1.961 MHz, 10 V on the end segments.',
  },
] as const;

/** Fetch an example's bundle (for api.importProject). */
export async function fetchExample(name: string): Promise<File> {
  const r = await fetch(`${import.meta.env.BASE_URL}examples/${name}.tar`);
  if (!r.ok) throw new Error(`example ${name}: HTTP ${r.status}`);
  return new File([await r.arrayBuffer()], `${name}.tar`, { type: 'application/x-tar' });
}
