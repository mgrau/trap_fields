//! Rigid-body transforms applied to the imported model (re-orienting the trap so its
//! axes line up with the simulation frame). The OCC model stays in the STEP frame;
//! everything we hand out (tessellation, meshes, coordinates) is mapped through the
//! composed transform.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TransformOp {
    /// Rotate by `angle_deg` (right-hand rule) about `axis` through `center` (m).
    Rotate {
        axis: [f64; 3],
        angle_deg: f64,
        #[serde(default)]
        center: [f64; 3],
    },
    /// Translate by `delta` (m).
    Translate { delta: [f64; 3] },
}

/// x' = R x + t
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rigid {
    pub r: [[f64; 3]; 3],
    pub t: [f64; 3],
}

impl Default for Rigid {
    fn default() -> Self {
        Self::IDENTITY
    }
}

fn matvec(r: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
}

fn matmul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

impl Rigid {
    pub const IDENTITY: Rigid = Rigid { r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

    pub fn from_op(op: &TransformOp) -> Rigid {
        match *op {
            TransformOp::Translate { delta } => Rigid { r: Self::IDENTITY.r, t: delta },
            TransformOp::Rotate { axis, angle_deg, center } => {
                let n = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
                if n == 0.0 {
                    return Self::IDENTITY;
                }
                let [x, y, z] = axis.map(|a| a / n);
                let th = angle_deg.to_radians();
                let (s, c) = th.sin_cos();
                // snap to exact values for multiples of 90 degrees
                let snap = |v: f64| if v.abs() < 1e-15 { 0.0 } else { v };
                let (s, c) = (snap(s), snap(c));
                let k = 1.0 - c;
                let r = [
                    [c + x * x * k, x * y * k - z * s, x * z * k + y * s],
                    [y * x * k + z * s, c + y * y * k, y * z * k - x * s],
                    [z * x * k - y * s, z * y * k + x * s, c + z * z * k],
                ];
                let rc = matvec(&r, center);
                Rigid { r, t: [center[0] - rc[0], center[1] - rc[1], center[2] - rc[2]] }
            }
        }
    }

    pub fn from_ops(ops: &[TransformOp]) -> Rigid {
        ops.iter().fold(Self::IDENTITY, |acc, op| acc.then(&Rigid::from_op(op)))
    }

    /// Apply `self` first, then `next`.
    pub fn then(&self, next: &Rigid) -> Rigid {
        let r = matmul(&next.r, &self.r);
        let t = matvec(&next.r, self.t);
        Rigid { r, t: [t[0] + next.t[0], t[1] + next.t[1], t[2] + next.t[2]] }
    }

    pub fn inverse(&self) -> Rigid {
        let rt = [0, 1, 2].map(|i| [0, 1, 2].map(|j| self.r[j][i]));
        let t = matvec(&rt, self.t);
        Rigid { r: rt, t: t.map(|v| -v) }
    }

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let v = matvec(&self.r, p);
        [v[0] + self.t[0], v[1] + self.t[1], v[2] + self.t[2]]
    }

    /// Transform a flat xyz array in place.
    pub fn apply_flat(&self, xyz: &mut [f64]) {
        for p in xyz.chunks_exact_mut(3) {
            let q = self.apply([p[0], p[1], p[2]]);
            p.copy_from_slice(&q);
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-12)
    }

    #[test]
    fn rotate_90_about_z() {
        let r = Rigid::from_op(&TransformOp::Rotate { axis: [0., 0., 1.], angle_deg: 90., center: [0.; 3] });
        assert_eq!(r.apply([1., 0., 0.]), [0., 1., 0.]);
    }

    #[test]
    fn rotate_about_center_and_compose_inverse() {
        let ops = vec![
            TransformOp::Rotate { axis: [1., 1., 0.], angle_deg: 37., center: [1., 2., 3.] },
            TransformOp::Translate { delta: [0.5, -1., 2.] },
            TransformOp::Rotate { axis: [0., 1., 0.], angle_deg: -90., center: [0.; 3] },
        ];
        let t = Rigid::from_ops(&ops);
        let p = [0.3, -0.7, 1.9];
        let mut q = p;
        for op in &ops {
            q = Rigid::from_op(op).apply(q);
        }
        assert!(close(t.apply(p), q));
        assert!(close(t.inverse().apply(t.apply(p)), p));
        // the center of rotation is a fixed point
        let r = Rigid::from_op(&ops[0]);
        assert!(close(r.apply([1., 2., 3.]), [1., 2., 3.]));
    }
}
