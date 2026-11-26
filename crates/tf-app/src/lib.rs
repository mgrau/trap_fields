//! trapfields application core, shared by the native server (`tf-server`) and the
//! WebAssembly engine (`tf-wasm`).
//!
//! - [`app`]: [`App`] holds the project and model and runs every operation
//! - [`project`]: `trapfields.json` (STEP file, transforms, labels, parameters)
//! - [`solve`]: BEM solve, saved solutions, trap analysis
//! - [`export`]: potentials and fields on a grid, as CSV
//! - [`moves`]: rigid displacements of single electrodes (what-if solves)
//! - [`tolerance`]: sensitivity of the trap to electrode displacements
//! - [`demo`]: generated traps: the needle demo and the examples/ projects (published Paul, surface and segmented traps)

pub mod app;
pub mod bundle;
pub mod demo;
pub mod export;
pub mod moves;
pub mod project;
pub mod solve;
pub mod tolerance;

pub use app::{App, AppError, AppResult, Event, JobStatus, Reply, Sink};
