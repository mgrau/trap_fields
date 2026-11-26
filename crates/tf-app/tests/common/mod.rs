//! Helpers shared by the example-project tests.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use tf_app::demo::Design;
use tf_app::{App, Event, Reply};

pub const E: f64 = 1.602176634e-19;
pub const AMU: f64 = 1.66053906660e-27;

pub fn cmd(app: &mut App, name: &str, args: Value) -> Value {
    let sink = |_e: Event| {};
    match app.command(name, args, None, &sink) {
        Ok(Reply::Json(v)) => v,
        Ok(Reply::Binary(_)) => panic!("{name}: binary reply"),
        Err(e) => panic!("{name}: {} ({})", e.message, e.status),
    }
}

pub fn example(design: Design) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(format!("{}_trap", design.name()))
}

pub fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("trapfields-examples-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A scratch copy of an example project (its STEP file and project file only).
pub fn copy_example(design: Design) -> PathBuf {
    let (src, dst) = (example(design), temp(design.name()));
    std::fs::create_dir_all(&dst).unwrap();
    for f in [design.step_file(), "trapfields.json"] {
        std::fs::copy(src.join(f), dst.join(f)).unwrap_or_else(|e| panic!("{}: {e}", src.join(f).display()));
    }
    dst
}

/// (name, electrode, bbox) of every body.
pub fn bodies(dir: &Path) -> Vec<(String, String, Vec<f64>)> {
    let mut app = App::new(dir).unwrap();
    let st = cmd(&mut app, "startup", Value::Null);
    st["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let bb = b["bbox"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            (b["name"].as_str().unwrap().to_string(), b["label"]["electrode"].as_str().unwrap_or("").to_string(), bb)
        })
        .collect()
}

