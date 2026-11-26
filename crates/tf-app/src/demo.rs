//! Ready-to-run example projects, generated (no CAD file needed), with electrodes
//! labeled and a drive set. Used by `trapfields demo`, by the example projects under
//! `examples/` and by the end-to-end tests.
//!
//! - [`Design::Linear`]: a linear Paul trap of four round rods and two needle endcaps.
//! - [`Design::Paul`]: the three-dimensional quadrupole ion trap of R. E. March's worked
//!   example (J. Mass Spectrom. 32, 351 (1997)): hyperboloid ring (r₀ = 1.00 cm) and
//!   endcaps, the endcaps moved out to z₀ = 0.783 cm (the "stretched" commercial trap),
//!   757 V (0-p) at 1.05 MHz on the ring, an ion of m/z 134.
//! - [`Design::PaulIdeal`]: the same trap unstretched, r₀² = 2z₀² (W. Paul, Rev. Mod.
//!   Phys. 62, 531 (1990)): with ideal (infinite) electrodes the potential inside is
//!   exactly φ = V/2 + V(r² − 2z²)/(2r₀²) for the ring at V and the endcaps grounded.
//! - [`Design::Surface`]: the asymmetric five-wire surface-electrode trap of Daniilidis
//!   et al., New J. Phys. 13, 013032 (2011): RF rails 400 µm and 200 µm wide, a 250 µm
//!   center electrode, 10 pairs of 1000 µm × 400 µm outer segments, 10 µm gaps, 5 µm
//!   thick gold, ion about 240 µm above the surface, ⁴⁰Ca⁺ at 15 MHz (85.9 V, the paper's
//!   "about 100 V" fitted to its frequencies).

use crate::project::{ExampleInfo, Label, Project, Role};
use crate::solve::{SolveParams, TrapParams};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f64::consts::{PI, SQRT_2};
use std::path::Path;
use tf_bem::trap::Ion;
use tf_geom::MeshParams;

/// STEP file of the linear trap (kept for callers of the original demo).
pub const STEP_FILE: &str = "linear_trap.step";

const MM: f64 = 1e-3;
const UM: f64 = 1e-6;
const CA40: Ion = Ion { mass_amu: 39.96259, charge_e: 1.0 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Design {
    #[default]
    Linear,
    Paul,
    Surface,
    Segmented,
    PaulIdeal,
}

impl Design {
    pub const ALL: [Design; 5] = [Design::Linear, Design::Paul, Design::Surface, Design::Segmented, Design::PaulIdeal];
    /// The designs shipped as example projects (each reproducing a published trap).
    pub const EXAMPLES: [Design; 3] = [Design::Paul, Design::Surface, Design::Segmented];

    pub fn name(self) -> &'static str {
        match self {
            Design::Linear => "linear",
            Design::Paul => "paul",
            Design::Surface => "surface",
            Design::Segmented => "segmented",
            Design::PaulIdeal => "paul-ideal",
        }
    }

    pub fn parse(s: &str) -> Option<Design> {
        Design::ALL.into_iter().find(|d| d.name() == s)
    }

    pub fn step_file(self) -> &'static str {
        match self {
            Design::Linear => STEP_FILE,
            Design::Paul => "paul_trap.step",
            Design::Surface => "surface_trap.step",
            Design::Segmented => "segmented_trap.step",
            Design::PaulIdeal => "paul_ideal_trap.step",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Design::Linear => "linear Paul trap: four rods and two needle endcaps",
            Design::Paul => "stretched hyperbolic ion trap of March's worked example, J. Mass Spectrom. 32, 351 (1997)",
            Design::PaulIdeal => "ideal hyperbolic 3D Paul trap, r0 = 1 cm, r0² = 2 z0²",
            Design::Surface => "five-wire surface trap after Daniilidis et al., NJP 13, 013032 (2011)",
            Design::Segmented => "segmented four-rod linear trap after Miao et al., arXiv:2201.03377 (2022)",
        }
    }

    /// Electrode and role of each body, in the order the bodies are created (gmsh names
    /// them Body 1, 2, …).
    pub fn electrodes(self) -> Vec<(String, Role)> {
        match self {
            Design::Linear => LINEAR.iter().map(|&(e, r)| (e.to_string(), r)).collect(),
            Design::Paul | Design::PaulIdeal => vec![("ring".into(), Role::Rf), ("cap+".into(), Role::Dc), ("cap-".into(), Role::Dc)],
            Design::Surface => {
                let mut v: Vec<(String, Role)> = vec![("RF".into(), Role::Rf), ("RF".into(), Role::Rf), ("C".into(), Role::Dc)];
                for side in ["W", "N"] {
                    for i in 1..=SURF_SEGMENTS {
                        v.push((format!("{side}{i}"), Role::Dc));
                    }
                }
                v
            }
            Design::Segmented => {
                let mut v = Vec::new();
                for seg in ["A", "B", "C"] {
                    for rod in 1..=4 {
                        let (e, role) = match (seg, rod) {
                            ("B", 2 | 4) => ("RF".to_string(), Role::Rf),
                            ("B", _) => ("B1, B3".to_string(), Role::Dc),
                            _ => (format!("{seg} (end)"), Role::Dc),
                        };
                        v.push((e, role));
                    }
                }
                v
            }
        }
    }

    /// Write the geometry as STEP (mm). Starts gmsh itself: no other gmsh user (such as
    /// an [`App`](crate::App)) may be alive.
    pub fn write_step(self, path: &Path) -> anyhow::Result<()> {
        let mut g = tf_gmsh::Gmsh::initialize()?;
        match self {
            Design::Linear => linear_geometry(&mut g)?,
            Design::Paul => paul_geometry(&mut g, MARCH_Z0)?,
            Design::PaulIdeal => paul_geometry(&mut g, PAUL_R0 / SQRT_2)?,
            Design::Surface => surface_geometry(&mut g)?,
            Design::Segmented => segmented_geometry(&mut g)?,
        }
        g.occ_synchronize()?;
        g.write(path)?;
        Ok(())
    }

    /// The published design the example reproduces, for the note on the Geometry tab.
    pub fn example_info(self) -> Option<ExampleInfo> {
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        match self {
            Design::Linear | Design::PaulIdeal => None,
            Design::Paul => Some(ExampleInfo {
                title: "Stretched quadrupole ion trap (3D Paul trap)".into(),
                reference: "R. E. March, \"An introduction to quadrupole ion trap mass spectrometry\", J. Mass Spectrom. 32, 351–369 (1997), worked example (Calculations)".into(),
                url: "https://doi.org/10.1002/(SICI)1096-9888(199704)32:4%3C351::AID-JMS512%3E3.0.CO;2-Y".into(),
                summary: "Hyperboloid ring and endcaps of the commercial ion trap, with the endcaps moved out by 10.6 % (\"stretched\"), driven as in the paper's worked example for butylbenzene (m/z 134). Compare the Trap tab's q_z and axial frequency with the paper's.".into(),
                published: pairs(&[
                    ("Electrodes", "ring r₀ = 1.00 cm; endcaps at z₀ = 0.783 cm (2z₀ = 15.66 mm), electrode shapes those of the unstretched trap"),
                    ("RF", "V = 757 V (0-p) on the ring at Ω/2π = 1.05 MHz, U = 0, endcaps grounded"),
                    ("Ion", "butylbenzene, m/z 134"),
                    ("Mathieu q", "q_z = 8eV/(m(r₀² + 2z₀²)Ω²) = 0.450 (a_z = 0)"),
                    ("β_z", "0.318 from β_z ≈ q_z/√2, \"high by about 5 %\" at this q"),
                    ("Axial frequency", "ω_z = β_zΩ/2 = 167 kHz, \"high by about 5 %\""),
                    ("Axial well depth", "D̄_z ≈ q_zV/8 = 43 V"),
                ]),
                assumptions: strings(&[
                    "Electrode truncation: ring hyperboloid out to |z| = r₀, endcaps out to the same radius (not stated).",
                    "The endcaps are the unstretched hyperboloids 2z² − r² = r₀² moved out along z by 0.783 − 0.707 = 0.076 cm.",
                ]),
            }),
            Design::Surface => Some(ExampleInfo {
                title: "Five-wire surface-electrode trap".into(),
                reference: "N. Daniilidis et al., \"Fabrication and heating rate study of microscopic surface electrode ion traps\", New J. Phys. 13, 013032 (2011)".into(),
                url: "https://arxiv.org/abs/1009.2834".into(),
                summary: "Gold-on-sapphire asymmetric five-wire trap with ⁴⁰Ca⁺ at 15 MHz. The RF amplitude and the DC set are fitted to the paper's secular frequencies and axis tilt. Compare them with the Trap tab.".into(),
                published: pairs(&[
                    ("Electrodes", "RF rails 400 and 200 µm, center 250 µm, 10 pairs of 1000 × 400 µm segments, 10 µm gaps, 5 µm thick"),
                    ("Ion", "⁴⁰Ca⁺, about 240 µm above the surface"),
                    ("RF", "about 100 V at Ω/2π ≈ 15 MHz"),
                    ("DC", "between −10 V and 15 V (no set given)"),
                    ("Secular frequencies", "(horizontal, vertical, axial) ≈ (1.2, 1.4, 0.4) MHz"),
                    ("RF quadrupole axes", "tilted about 25° from vertical"),
                ]),
                assumptions: strings(&[
                    "RF 85.9 V rather than about 100 V: with only the RF setting the sum of the three squared frequencies, the paper's (1.2, 1.4, 0.4) MHz needs 85.9 V in this geometry.",
                    "DC set (not published, all within the paper's −10…15 V): fitted to cancel the static field at the RF null and give the paper's three frequencies with the axes tilted 25°. Center +0.22 V; segments 1–5 (mirrored for 6–10), wide side −3.85, −3.52, −4.37, −3.70, +2.13 V; narrow side −3.24, −2.38, −0.63, +11.16, −2.31 V.",
                    "The rails and center electrode run 500 µm past the segment rows; no ground plane or substrate.",
                ]),
            }),
            Design::Segmented => Some(ExampleInfo {
                title: "Segmented four-rod linear trap".into(),
                reference: "S. N. Miao et al., \"Sympathetic cooling of a large ¹¹³Cd⁺ ion crystal with ⁴⁰Ca⁺ in a linear Paul trap\", arXiv:2201.03377 (2022)".into(),
                url: "https://arxiv.org/abs/2201.03377".into(),
                summary: "Four round rods (R/r₀ = 1.145), each cut into end segments A, C and a trapping segment B. The drive is the paper's typical one for ⁴⁰Ca⁺. Compare the Trap tab's Mathieu q and a with the paper's.".into(),
                published: pairs(&[
                    ("Electrodes", "rod diameter 14.2 mm, r₀ = 6.2 mm; segments A and C 20 mm, B 2z₀ = 40 mm"),
                    ("RF", "U_rf = 150 V on rods B2 and B4, Ω/2π = 1.961 MHz"),
                    ("DC", "U_end = 10 V on the end segments A and C"),
                    ("Ion", "⁴⁰Ca⁺ (and ¹¹³Cd⁺, q_Cd = 0.044)"),
                    ("Mathieu q", "q_Ca = 2QU_rf/(MΩ²r₀²) = 0.124"),
                    ("Mathieu a", "a_Ca ≈ 0"),
                    ("Axial factor", "κ = 0.12, with a = 4QκU_end/(MΩ²z₀²), set in their simulation by matching simulated crystal images to experiment"),
                    ("Trap depth", "should exceed 3 eV for stable trapping (a general requirement, not computed for this drive)"),
                ]),
                assumptions: strings(&[
                    "1 mm gaps between the segments (not stated).",
                    "Rods B1 and B3 grounded; the paper does not say. With ±75 V on the two pairs instead, the depth is 2.1 eV rather than 1.6 eV.",
                ]),
            }),
        }
    }

    /// The drive the example comes with.
    pub fn drive(self) -> TrapParams {
        let mut rf = BTreeMap::new();
        let mut dc = BTreeMap::new();
        let (rf_freq, ion) = match self {
            Design::Linear => {
                for (name, role) in LINEAR {
                    let rod = role == Role::Rf;
                    rf.insert(name.to_string(), if !rod { 0.0 } else if name == "rod ++" || name == "rod --" { 150.0 } else { -150.0 });
                    dc.insert(name.to_string(), if rod { 0.0 } else { 5.0 });
                }
                (20e6, CA40)
            }
            Design::Paul | Design::PaulIdeal => {
                let (v, f, ion) = if self == Design::Paul { (MARCH_V, MARCH_FREQ, MARCH_ION) } else { (PAUL_V, PAUL_FREQ, CA40) };
                rf.insert("ring".into(), v);
                for e in ["ring", "cap+", "cap-"] {
                    rf.entry(e.into()).or_insert(0.0);
                    dc.insert(e.into(), 0.0);
                }
                (f, ion)
            }
            Design::Surface => {
                rf.insert("RF".into(), SURF_V);
                dc.insert("RF".into(), 0.0);
                rf.insert("C".into(), 0.0);
                dc.insert("C".into(), SURF_DC_CENTER);
                for (side, set) in [("W", SURF_DC_WIDE), ("N", SURF_DC_NARROW)] {
                    for i in 1..=SURF_SEGMENTS {
                        rf.insert(format!("{side}{i}"), 0.0);
                        dc.insert(format!("{side}{i}"), set[i.min(SURF_SEGMENTS + 1 - i) - 1]);
                    }
                }
                (SURF_FREQ, CA40)
            }
            Design::Segmented => {
                for (e, _) in self.electrodes() {
                    let rf_v = if e == "RF" { SEG_V } else { 0.0 };
                    let dc_v = if e.ends_with("(end)") { SEG_END } else { 0.0 };
                    rf.insert(e.clone(), rf_v);
                    dc.insert(e, dc_v);
                }
                (SEG_FREQ, CA40)
            }
        };
        TrapParams { ion, rf_freq, rf, dc, depth: false }
    }

    pub fn mesh_params(self) -> MeshParams {
        match self {
            Design::Linear => MeshParams { center: [0.0; 3], h_min: 0.06 * MM, h_max: 1.2 * MM, dist_min: 0.7 * MM, dist_max: 8.0 * MM, min_circle_points: 12 },
            // fine out to the ring (r₀ = 10 mm): the depth is reached on its surface
            Design::Paul | Design::PaulIdeal => MeshParams { center: [0.0; 3], h_min: 0.6 * MM, h_max: 3.0 * MM, dist_min: 10.5 * MM, dist_max: 25.0 * MM, min_circle_points: 24 },
            Design::Segmented => MeshParams { center: [0.0; 3], h_min: 0.5 * MM, h_max: 4.0 * MM, dist_min: 7.0 * MM, dist_max: 40.0 * MM, min_circle_points: 24 },
            Design::Surface => MeshParams {
                center: [0.0, SURF_HEIGHT, 0.0],
                h_min: 20.0 * UM,
                h_max: 400.0 * UM,
                dist_min: 400.0 * UM,
                dist_max: 3.0 * MM,
                min_circle_points: 12,
            },
        }
    }
}

// ---------------------------------------------------------------------------------------
// linear trap

const LINEAR: [(&str, Role); 6] = [("rod ++", Role::Rf), ("rod -+", Role::Rf), ("rod --", Role::Rf), ("rod +-", Role::Rf), ("EC+", Role::Dc), ("EC-", Role::Dc)];

/// Electrode of each body of the linear trap, in creation order.
pub const ELECTRODES: [(&str, Role); 6] = LINEAR;

/// Rods along z at 45° (axes 1.2 mm from the trap axis, radius 0.5 mm, 10 mm long),
/// needles on the axis from |z| = 2.5 mm out to 5 mm (radius 0.2 mm).
fn linear_geometry(g: &mut tf_gmsh::Gmsh) -> anyhow::Result<()> {
    let d = 1.2 / SQRT_2;
    for (x, y) in [(d, d), (-d, d), (-d, -d), (d, -d)] {
        g.occ_add_cylinder([x, y, -5.0], [0.0, 0.0, 10.0], 0.5)?;
    }
    g.occ_add_cylinder([0.0, 0.0, 2.5], [0.0, 0.0, 2.5], 0.2)?;
    g.occ_add_cylinder([0.0, 0.0, -2.5], [0.0, 0.0, -2.5], 0.2)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------
// hyperbolic Paul trap

/// Ring radius r₀; the unstretched endcaps are at z₀ = r₀/√2.
pub const PAUL_R0: f64 = 10.0 * MM;
/// March's worked example: endcaps at z₀ = 0.783 cm, 757 V (0-p) at 1.05 MHz, m/z 134.
pub const MARCH_Z0: f64 = 7.83 * MM;
pub const MARCH_V: f64 = 757.0;
pub const MARCH_FREQ: f64 = 1.05e6;
pub const MARCH_ION: Ion = Ion { mass_amu: 134.0, charge_e: 1.0 };
/// Ring hyperboloid kept out to |z| = r₀ (r = √3 r₀); endcaps out to the same radius.
const PAUL_TRUNC: f64 = 1.0;
/// The ideal trap's drive (V, Hz): q_z ≈ 0.49 for ⁴⁰Ca⁺.
pub const PAUL_V: f64 = 200.0;
pub const PAUL_FREQ: f64 = 1e6;

/// Profile in the x–z plane revolved about the z axis: the ring between its hyperboloid
/// r² − 2z² = r₀² and an outer cylinder, the endcaps between their hyperboloids
/// 2z² − r² = r₀² (moved out along z so their vertices are at ±`z0_end`) and flat backs.
fn paul_geometry(g: &mut tf_gmsh::Gmsh, z0_end: f64) -> anyhow::Result<()> {
    let r0 = PAUL_R0 / MM;
    let z0 = r0 / SQRT_2;
    let shift = z0_end / MM - z0;
    let zt = PAUL_TRUNC * r0;
    let rt = (r0 * r0 + 2.0 * zt * zt).sqrt();
    let r_out = rt + 0.3 * r0;
    const N: usize = 40;
    let revolve = |g: &mut tf_gmsh::Gmsh, curves: Vec<i32>| -> anyhow::Result<()> {
        let lp = g.occ_add_curve_loop(&curves)?;
        let s = g.occ_add_plane_surface(&[lp])?;
        g.occ_revolve(&[(2, s)], [0.0; 3], [0.0, 0.0, 1.0], 2.0 * PI)?;
        Ok(())
    };
    // ring
    let pts: Vec<i32> = (0..=N)
        .map(|i| {
            let z = -zt + 2.0 * zt * i as f64 / N as f64;
            g.occ_add_point([(r0 * r0 + 2.0 * z * z).sqrt(), 0.0, z])
        })
        .collect::<Result<_, _>>()?;
    let hyp = g.occ_add_spline(&pts)?;
    let (a, b) = (g.occ_add_point([r_out, 0.0, zt])?, g.occ_add_point([r_out, 0.0, -zt])?);
    let curves = vec![hyp, g.occ_add_line(pts[N], a)?, g.occ_add_line(a, b)?, g.occ_add_line(b, pts[0])?];
    revolve(g, curves)?;
    // endcaps, +z first
    let back = shift + (z0 * z0 + rt * rt / 2.0).sqrt() + 0.3 * r0;
    for s in [1.0, -1.0] {
        let pts: Vec<i32> = (0..=N)
            .map(|i| {
                let r = rt * i as f64 / N as f64;
                g.occ_add_point([r, 0.0, s * (shift + (z0 * z0 + r * r / 2.0).sqrt())])
            })
            .collect::<Result<_, _>>()?;
        let hyp = g.occ_add_spline(&pts)?;
        let (a, b) = (g.occ_add_point([rt, 0.0, s * back])?, g.occ_add_point([0.0, 0.0, s * back])?);
        let curves = vec![hyp, g.occ_add_line(pts[N], a)?, g.occ_add_line(a, b)?, g.occ_add_line(b, pts[0])?];
        revolve(g, curves)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// surface trap

/// Daniilidis et al. (2011): widths across the trap axis (x), µm.
pub const SURF_RF_WIDE: f64 = 400.0 * UM;
pub const SURF_RF_NARROW: f64 = 200.0 * UM;
pub const SURF_CENTER: f64 = 250.0 * UM;
pub const SURF_GAP: f64 = 10.0 * UM;
pub const SURF_THICK: f64 = 5.0 * UM;
/// Outer segments: 1000 µm across, 400 µm along the axis, 10 on each side.
pub const SURF_SEG_W: f64 = 1000.0 * UM;
pub const SURF_SEG_L: f64 = 400.0 * UM;
pub const SURF_SEGMENTS: usize = 10;
/// Published ion height (approximate) and drive.
pub const SURF_HEIGHT: f64 = 240.0 * UM;
/// RF amplitude: the paper says "about 100 V". Laplace's equation fixes the sum of the
/// three squared secular frequencies by the RF alone, whatever the DC set, and the paper's
/// (1.2, 1.4, 0.4) MHz needs 85.9 V in this geometry (at 100 V the sum is 33 % too large).
pub const SURF_V: f64 = 85.9;
pub const SURF_FREQ: f64 = 15e6;
/// DC set (not published: the paper quotes only the range −10…15 V), fitted so that the
/// static field vanishes at the RF null and the exact secular frequencies are the paper's
/// (1.2, 1.4, 0.4) MHz with the radial axes tilted 25° from vertical, as reported. The
/// minimum-norm solution, symmetric about the trap center: the center electrode, then
/// segments 1…5 on each side (segment 11 − k is at the same voltage as segment k) (V).
pub const SURF_DC_CENTER: f64 = 0.22;
pub const SURF_DC_WIDE: [f64; 5] = [-3.85, -3.52, -4.37, -3.70, 2.13];
pub const SURF_DC_NARROW: [f64; 5] = [-3.24, -2.38, -0.63, 11.16, -2.31];

/// Left and right edges (x) of the strips across the chip, from the narrow side:
/// narrow segments, narrow RF, center, wide RF, wide segments.
pub fn surface_strips() -> [(f64, f64); 5] {
    let (g, c) = (SURF_GAP, SURF_CENTER / 2.0);
    let rf_n = (-c - g - SURF_RF_NARROW, -c - g);
    let rf_w = (c + g, c + g + SURF_RF_WIDE);
    let seg_n = (rf_n.0 - g - SURF_SEG_W, rf_n.0 - g);
    let seg_w = (rf_w.1 + g, rf_w.1 + g + SURF_SEG_W);
    [seg_n, rf_n, (-c, c), rf_w, seg_w]
}

/// Electrodes as 5 µm thick boxes with their top faces in the plane y = 0, the trap axis
/// along z. The RF rails and the center electrode run 500 µm past the segment rows at
/// each end. (The paper gives no overall extent, the length of the center electrode or
/// the surrounding ground plane: those are left out or assumed here.)
fn surface_geometry(g: &mut tf_gmsh::Gmsh) -> anyhow::Result<()> {
    let s = surface_strips();
    let pitch = SURF_SEG_L + SURF_GAP;
    let rows = SURF_SEGMENTS as f64 * pitch - SURF_GAP;
    let len = rows + 1000.0 * UM;
    let mm = |v: f64| v / MM;
    let strip = |g: &mut tf_gmsh::Gmsh, (x0, x1): (f64, f64), z0: f64, l: f64| g.occ_add_box([mm(x0), mm(-SURF_THICK), mm(z0)], [mm(x1 - x0), mm(SURF_THICK), mm(l)]);
    strip(g, s[3], -len / 2.0, len)?; // RF, wide
    strip(g, s[1], -len / 2.0, len)?; // RF, narrow
    strip(g, s[2], -len / 2.0, len)?; // center
    for side in [s[4], s[0]] {
        for i in 0..SURF_SEGMENTS {
            strip(g, side, -rows / 2.0 + i as f64 * pitch, SURF_SEG_L)?;
        }
    }
    Ok(())
}

/// The five-wire layout in the gapless-plane model (an infinite plane of electrodes, each
/// gap split between its neighbors): the RF rails' edges (x), from [`surface_strips`].
pub fn gapless_rails() -> [(f64, f64); 2] {
    let s = surface_strips();
    let g = SURF_GAP / 2.0;
    [(s[1].0 - g, s[1].1 + g), (s[3].0 - g, s[3].1 + g)]
}

/// Potential at (x, y) (y above the plane) of strips at 1 V in an otherwise grounded,
/// gapless plane: φ = (1/π) Σ [atan((x − x₀)/y) − atan((x − x₁)/y)].
pub fn gapless_phi(rails: &[(f64, f64)], x: f64, y: f64) -> f64 {
    rails.iter().map(|&(x0, x1)| (((x - x0) / y).atan() - ((x - x1) / y).atan()) / PI).sum()
}

/// The RF null of strips in the gapless plane (x, y), by Newton iteration from `guess`.
pub fn gapless_null(rails: &[(f64, f64)], guess: [f64; 2]) -> [f64; 2] {
    // ∇φ of a strip edge at e (sign s): ∂x = s·y/(π d), ∂y = −s·(x − e)/(π d), d = (x − e)² + y²
    let grad = |p: [f64; 2]| -> [f64; 2] {
        let mut g = [0.0; 2];
        for &(x0, x1) in rails {
            for (e, s) in [(x0, 1.0), (x1, -1.0)] {
                let (u, y) = (p[0] - e, p[1]);
                let d = (u * u + y * y) * PI;
                g[0] += s * y / d;
                g[1] -= s * u / d;
            }
        }
        g
    };
    let mut p = guess;
    for _ in 0..100 {
        let g0 = grad(p);
        let h = 1e-6 * p[1].abs().max(1e-12);
        let (gx, gy) = (grad([p[0] + h, p[1]]), grad([p[0], p[1] + h]));
        let j = [[(gx[0] - g0[0]) / h, (gy[0] - g0[0]) / h], [(gx[1] - g0[1]) / h, (gy[1] - g0[1]) / h]];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        let dx = (g0[0] * j[1][1] - g0[1] * j[0][1]) / det;
        let dy = (g0[1] * j[0][0] - g0[0] * j[1][0]) / det;
        p = [p[0] - dx, p[1] - dy];
        if dx.hypot(dy) < 1e-12 * p[1].abs() {
            break;
        }
    }
    p
}

// ---------------------------------------------------------------------------------------
// segmented four-rod trap (Miao et al. 2022)

/// Rod diameter 14.2 mm, r₀ = 6.2 mm (center to rod surface): R/r₀ = 1.145.
pub const SEG_ROD_R: f64 = 7.1 * MM;
pub const SEG_R0: f64 = 6.2 * MM;
/// Trapping segments B: 2z₀ = 40 mm; end segments A and C: 20 mm each.
pub const SEG_Z0: f64 = 20.0 * MM;
pub const SEG_END_LEN: f64 = 20.0 * MM;
/// Gap between segments (not stated in the paper: assumed).
pub const SEG_GAP: f64 = 1.0 * MM;
/// Typical operating point: U_rf = 150 V on rods B2 and B4 at Ω/2π = 1.961 MHz,
/// U_end = 10 V on the end segments; q_Ca = 0.124, a ≈ 0.
pub const SEG_V: f64 = 150.0;
pub const SEG_FREQ: f64 = 1.961e6;
pub const SEG_END: f64 = 10.0;
/// Axial geometry factor the paper uses: ω_z² = 2κ e U_end/(m z₀²).
pub const SEG_KAPPA: f64 = 0.12;

/// Four round rods along z at 45° (rod 1 at +x+y, then counterclockwise), each cut into
/// segments A (z > 0 end), B (middle) and C, created A1…A4, B1…B4, C1…C4.
fn segmented_geometry(g: &mut tf_gmsh::Gmsh) -> anyhow::Result<()> {
    let d = (SEG_R0 + SEG_ROD_R) / SQRT_2 / MM;
    let (z0, l, gap, r) = (SEG_Z0 / MM, SEG_END_LEN / MM, SEG_GAP / MM, SEG_ROD_R / MM);
    for (z, len) in [(z0 + gap, l), (-z0, 2.0 * z0), (-z0 - gap - l, l)] {
        for (x, y) in [(d, d), (-d, d), (-d, -d), (d, -d)] {
            g.occ_add_cylinder([x, y, z], [0.0, 0.0, len], r)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------

/// Write the linear trap's STEP file (see [`Design::write_step`]).
pub fn write_step(path: &Path) -> anyhow::Result<()> {
    Design::Linear.write_step(path)
}

/// The linear trap's drive.
pub fn drive() -> TrapParams {
    Design::Linear.drive()
}

/// Write the linear trap's project into `dir`.
pub fn create(dir: &Path) -> anyhow::Result<()> {
    create_design(dir, Design::Linear)
}

/// Write the STEP file and a project file (labels, mesh sizes, drive) into `dir`.
pub fn create_design(dir: &Path, design: Design) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    anyhow::ensure!(!Project::path(dir).exists(), "{} already has a project file", dir.display());
    design.write_step(&dir.join(design.step_file()))?;
    let labels = design.electrodes().into_iter().enumerate().map(|(i, (e, role))| (format!("Body {}", i + 1), Label { electrode: e, role })).collect();
    let project = Project {
        version: 1,
        step_file: Some(design.step_file().into()),
        labels,
        mesh_params: Some(design.mesh_params()),
        solve_params: Some(SolveParams { lmax: 8, ..Default::default() }),
        trap_params: Some(design.drive()),
        example: design.example_info(),
        ..Default::default()
    };
    project.save(dir)
}
