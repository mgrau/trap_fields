//! trapfields application core, shared by the native server (`tf-server`) and the
//! WebAssembly engine (`tf-wasm`).
//!
//! - [`app`]: [`App`] holds the project and model and runs every operation
//! - [`project`]: `trapfields.json` (STEP file, transforms, labels, parameters)
//! - [`solve`]: BEM solve, saved solutions, trap analysis

pub mod app;
pub mod project;
pub mod solve;

pub use app::{App, AppError, AppResult, Event, JobStatus, Reply, Sink};
