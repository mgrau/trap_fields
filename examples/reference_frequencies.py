#!/usr/bin/env python3
"""Reference secular frequencies and pseudopotential depths for the example traps, computed independently of the
BEM solver (numpy only). The numbers it prints are quoted in examples/README.md and
checked by crates/tf-app/tests/examples.rs.

- paul_trap: March's worked example (stretched trap): his q_z = 8eV/(m(r₀² + 2z₀²)Ω²),
  the lowest-order frequency q_zΩ/(2√2) he quotes, and the exact βΩ/2 from Mathieu's
  equation. Also the ideal unstretched trap the tests generate: q_z = 4eV/(mΩ²r₀²),
  q_r = −q_z/2.
- surface_trap: the RF-only radial frequency of the gapless-plane model (an infinite plane
  of strips, each gap split between its neighbors).
Depths (pseudopotential, RF only, e²|E|²/(4mΩ²)):

- ideal Paul trap: q_z V/16 at the ring (r = r₀), the lowest point of the ideal electrodes.
- surface_trap: the escape saddle of the gapless plane, from the trap to above it.
- segmented_trap, linear_trap: the saddle between neighboring rods (radial escape; the axial depth set by
  the needles has no reference).

- segmented_trap (and the needle demo, linear_trap): the radial frequency of four
  infinite round rods (2D, solved with
  circular-harmonic expansions on each rod), corrected for the endcaps' DC
  defocusing ω_r² = ω_p² − ω_z²/2 with the solved axial frequency.
"""
from math import atan, atan2, cos, hypot, log, pi, sin, sqrt, acos

import numpy as np

E = 1.602176634e-19
M = 39.96259 * 1.66053906660e-27  # 40Ca+


def mathieu_beta(a, q, n=4000):
    """Characteristic exponent β of x'' + (a − 2q cos 2τ) x = 0 (stable region), from the
    monodromy matrix over one period τ ∈ [0, π] (RK4)."""
    def f(t, y):
        return np.array([y[1], -(a - 2 * q * cos(2 * t)) * y[0]])

    cols = []
    for y0 in ([1.0, 0.0], [0.0, 1.0]):
        y, t, h = np.array(y0), 0.0, pi / n
        for _ in range(n):
            k1 = f(t, y)
            k2 = f(t + h / 2, y + h / 2 * k1)
            k3 = f(t + h / 2, y + h / 2 * k2)
            k4 = f(t + h, y + h * k3)
            y, t = y + h / 6 * (k1 + 2 * k2 + 2 * k3 + k4), t + h
        cols.append(y)
    return acos((cols[0][0] + cols[1][1]) / 2) / pi


def pseudo_ev(field2, freq):
    """Pseudopotential (eV) for a squared RF field amplitude |E|² (V²/m²)."""
    w = 2 * pi * freq
    return E * field2 / (4 * M * w * w)


def flood_depth(u, start, is_exit):
    """Lowest barrier (max of u along the route) from grid node `start` to any node with
    is_exit(j, i): a priority flood over the 8-neighbour grid."""
    import heapq

    ny, nx = u.shape
    done = np.zeros(u.shape, bool)
    heap = [(u[start], start)]
    while heap:
        c, (j, i) = heapq.heappop(heap)
        if done[j, i]:
            continue
        done[j, i] = True
        if is_exit(j, i):
            return c - u[start]
        for dj in (-1, 0, 1):
            for di in (-1, 0, 1):
                jj, ii = j + dj, i + di
                if 0 <= jj < ny and 0 <= ii < nx and not done[jj, ii] and np.isfinite(u[jj, ii]):
                    heapq.heappush(heap, (max(c, u[jj, ii]), (jj, ii)))
    return float("nan")


def quadrupole(curv, freq):
    """Pseudopotential and exact frequency (Hz) and q for an RF potential whose transverse
    curvature is ±curv (V/m², at the RF amplitude) at drive frequency freq."""
    w = 2 * pi * freq
    q = 2 * E * curv / (M * w * w)
    return q, q * freq / (2 * sqrt(2)), mathieu_beta(0.0, q) * freq / 2


def march():
    """March (1997) worked example: stretched trap, r0 = 1.00 cm, z0 = 0.783 cm, 757 V (0-p)
    at 1.05 MHz, m/z 134. His q_z formula and lowest-order β_z ≈ q_z/√2, and the exact β_z."""
    r0, z0, v, f, m = 0.01, 0.00783, 757.0, 1.05e6, 134.0 * 1.66053906660e-27
    w = 2 * pi * f
    qz = 8 * E * v / (m * (r0**2 + 2 * z0**2) * w * w)
    b = mathieu_beta(0.0, qz)
    print(f"paul_trap     (March) q_z {qz:.4f}  axial  q_z/√2 {qz / sqrt(2) * f / 2 / 1e3:8.2f} kHz  exact β_z = {b:.4f}: {b * f / 2 / 1e3:8.2f} kHz")
    print(f"              radial q_r {qz / 2:.4f}  pseudo {qz / 2 / sqrt(2) * f / 2 / 1e3:8.2f} kHz  exact {mathieu_beta(0.0, qz / 2) * f / 2 / 1e3:8.2f} kHz")
    print(f"              axial well depth D_z ≈ q_z V/8 = {qz * v / 8:.1f} eV")


def paul():
    """The ideal (unstretched) trap of the tests: r0² = 2 z0², 200 V at 1 MHz, 40Ca+."""
    r0, v, f = 0.01, 200.0, 1e6
    qz, fz, bz = quadrupole(4 * v / r0**2 / 2, f)  # φ_zz = −2V/r0², curvature 2V/r0²
    qr, fr, br = quadrupole(2 * v / r0**2 / 2, f)  # φ_rr = V/r0²
    print(f"ideal Paul    q_z {qz:.4f}  axial  pseudo {fz/1e3:8.2f} kHz  exact {bz/1e3:8.2f} kHz")
    print(f"              q_r {qr:.4f}  radial pseudo {fr/1e3:8.2f} kHz  exact {br/1e3:8.2f} kHz")
    print(f"              depth q_z V/16 = {qz * v / 16:.3f} eV (at the ring)")


def surface():
    # demo.rs: rails 200 µm (−335…−135) and 400 µm (135…535) wide, 10 µm gaps split
    rails = [(-340e-6, -130e-6), (130e-6, 540e-6)]

    def phi(x, y):
        return sum((atan((x - a) / y) - atan((x - b) / y)) / pi for a, b in rails)

    def grad(x, y, h=1e-9):
        return np.array([(phi(x + h, y) - phi(x - h, y)) / (2 * h), (phi(x, y + h) - phi(x, y - h)) / (2 * h)])

    p = np.array([0.0, 240e-6])
    for _ in range(50):  # Newton on ∇φ = 0
        h = 1e-8
        j = np.column_stack([(grad(*(p + [h, 0])) - grad(*(p - [h, 0]))) / (2 * h), (grad(*(p + [0, h])) - grad(*(p - [0, h]))) / (2 * h)])
        p = p - np.linalg.solve(j, grad(*p))
    h = 1e-7
    x, y = p
    pxx = (phi(x + h, y) + phi(x - h, y) - 2 * phi(x, y)) / h**2
    pyy = (phi(x, y + h) + phi(x, y - h) - 2 * phi(x, y)) / h**2
    pxy = (phi(x + h, y + h) - phi(x + h, y - h) - phi(x - h, y + h) + phi(x - h, y - h)) / (4 * h * h)
    lam = max(abs(np.linalg.eigvalsh([[pxx, pxy], [pxy, pyy]])))
    v = 85.9  # demo.rs SURF_V: fitted to the paper's frequencies (it says about 100 V)
    q, fp, fb = quadrupole(v * lam, 15e6)
    tilt = 0.5 * np.degrees(np.arctan2(2 * pxy, pxx - pyy))
    print(f"surface_trap  RF null ({x*1e6:.1f}, {y*1e6:.1f}) µm, axes tilted {abs(tilt) if abs(tilt) < 45 else 90 - abs(tilt):.1f}°")
    print(f"              RF only: q {q:.4f}  radial pseudo {fp/1e6:.4f} MHz  exact {fb/1e6:.4f} MHz")
    # depth: flood the cross-section from the null up to 1.4 mm (the field falls off as 1/y²)
    xs, ys = np.linspace(-700e-6, 700e-6, 281), np.linspace(20e-6, 1400e-6, 277)
    u = np.array([[pseudo_ev(v**2 * float(grad(xx, yy) @ grad(xx, yy)), 15e6) for xx in xs] for yy in ys])
    start = np.unravel_index(np.argmin(u), u.shape)
    d = flood_depth(u, start, lambda j, i: j == len(ys) - 1)
    print(f"              RF-only depth {d*1e3:.1f} meV (escape upward, cross-section)")


def rods(name, d, r, v, f, f_axial):
    """Four infinite round rods at (±d, ±d), radius r, diagonal pairs at ±v."""
    rods = [(d, d, v), (-d, d, -v), (-d, -d, v), (d, -d, -v)]
    n, k = 12, 64

    def basis(px, py):
        row = []
        for cx, cy, _ in rods:
            dx, dy = px - cx, py - cy
            rho, th = hypot(dx, dy), atan2(dy, dx)
            row.append(log(rho / r))
            for m in range(1, n + 1):
                row += [cos(m * th) * (r / rho) ** m, sin(m * th) * (r / rho) ** m]
        return row + [1.0]

    a, b = [], []
    for cx, cy, u in rods:
        for i in range(k):
            t = 2 * pi * i / k
            a.append(basis(cx + r * cos(t), cy + r * sin(t)))
            b.append(u)
    sol, *_ = np.linalg.lstsq(np.array(a), np.array(b), rcond=None)

    def phi(x, y):
        return np.array(basis(x, y)) @ sol

    h = 1e-6
    pxx = (phi(h, 0) + phi(-h, 0) - 2 * phi(0, 0)) / h**2
    pxy = (phi(h, h) - phi(h, -h) - phi(-h, h) + phi(-h, -h)) / (4 * h * h)
    q, fp, fb = quadrupole(max(abs(np.linalg.eigvalsh([[pxx, pxy], [pxy, -pxx]]))), f)
    # radial depth: by symmetry the saddle between neighboring rods is on the x axis
    xs = np.linspace(0.0, d + 0.3e-3, 2001)
    e2 = []
    for xx in xs:
        gx = (phi(xx + h, 0) - phi(xx - h, 0)) / (2 * h)
        gy = (phi(xx, h) - phi(xx, -h)) / (2 * h)
        e2.append(gx * gx + gy * gy)
    # the rod face nearest the axis, along the diagonal: lower than the saddle between rods
    t = (d * sqrt(2) - r - 1e-6) / sqrt(2)
    gx = (phi(t + h, t) - phi(t - h, t)) / (2 * h)
    gy = (phi(t, t + h) - phi(t, t - h)) / (2 * h)
    face = pseudo_ev(gx * gx + gy * gy, f)
    depth = f"              RF-only pseudopotential at the rod face {face:.3f} eV (harmonic estimate {abs(q) * v / 4:.3f} eV)\n"
    depth += f"              RF-only saddle between rods {pseudo_ev(max(e2), f):.3f} eV at x = {xs[int(np.argmax(e2))]*1e3:.3f} mm"
    corr = lambda fr: sqrt(fr**2 - f_axial**2 / 2)
    print(f"{name:14s}q {q:.4f}  radial pseudo {fp/1e3:8.2f} kHz  exact {fb/1e3:8.2f} kHz (RF only)")
    print(f"              with the endcaps (axial {f_axial/1e3:.1f} kHz): pseudo {corr(fp)/1e3:8.2f} kHz  exact {corr(fb)/1e3:8.2f} kHz")
    print(depth)


def segmented():
    # Miao et al.: rods 14.2 mm in diameter, r0 = 6.2 mm, 150 V on one diagonal pair and
    # the other grounded = ±75 V plus a common 75 V, which adds no quadrupole
    w = 2 * pi * 1.961e6
    print(f"segmented_trap  paper formula q = 2QU/(MΩ²r0²) = {2 * E * 150 / (M * w * w * 6.2e-3**2):.4f} (published 0.124)")
    rods("", 13.3e-3 / sqrt(2), 7.1e-3, 75.0, 1.961e6, 10.8e3)


def linear():
    rods("linear_trap", 1.2e-3 / sqrt(2), 0.5e-3, 150.0, 20e6, 48.2e3)


if __name__ == "__main__":
    march()
    paul()
    surface()
    segmented()
    linear()
