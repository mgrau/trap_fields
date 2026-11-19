//! Boundary-element electrostatics for ion-trap electrodes.
//!
//! - [`panel`]: flat P0 triangles, closed-form and quadrature single-layer integrals
//! - [`solver`]: collocation system, floating conductors, one basis solution per electrode
//! - [`harmonics`]: real solid harmonics and expansions computed directly from the charges

pub mod harmonics;
pub mod panel;
pub mod solver;
pub mod trap;

pub use harmonics::{Expansion, Local};
pub use panel::{Panel, V3};
pub use solver::{evaluate, expand, moment_table, solve, BemError, Conductor, Problem, Solution, Stage, Timings, EPS0};
