//! Trap geometry pipeline: STEP import with electrode names, rigid re-orientation,
//! display tessellation and graded surface meshing for the BEM solver.

pub mod mesh;
pub mod metrics;
pub mod model;
pub mod step;
pub mod transform;

pub use mesh::{mesh_bem, MeshParams, MeshStats, SurfaceMesh};
pub use model::{load_step, tessellate, Body, Face, GeomError, Model, Result, Tessellation};
pub use transform::{Rigid, TransformOp};
