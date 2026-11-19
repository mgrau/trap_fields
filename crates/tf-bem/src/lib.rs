//! Boundary-element electrostatics for ion-trap electrodes.
//!
//! - [`panel`]: flat P0 triangles, closed-form and quadrature single-layer integrals
//! - [`solver`]: collocation system, floating conductors, one basis solution per electrode
//! - [`harmonics`]: real solid harmonics and expansions computed directly from the charges
//! - [`hmatrix`], [`iterative`]: compressed (H-matrix/ACA) operator and GMRES for large problems
//! - [`trap`]: pseudopotential analysis

pub mod chain;
pub mod harmonics;
pub mod hmatrix;
pub mod iterative;
pub mod panel;
pub mod solver;
pub mod trap;

pub use harmonics::{Expansion, Local};
pub use panel::{Panel, V3};
pub use hmatrix::{HParams, HStats};
pub use iterative::GmresParams;
pub use solver::{evaluate, expand, moment_table, solve, solve_with, solve_with_stop, BemError, Conductor, Method, Problem, Progress, Solution, SolveOptions, SolverStats, Stage, Timings, EPS0};
