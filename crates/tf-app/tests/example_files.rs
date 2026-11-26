//! The example projects committed under `examples/` (STEP and project files) are what
//! their generators (`trapfields demo --design …`) write. In a test binary of its own:
//! gmsh cannot be started again to write STEP once an App has used it in the process.

mod common;

use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tf_app::demo::{self, Design};

#[test]
fn committed_examples_match_their_generators() {
    // generate everything first (see above)
    let fresh: Vec<PathBuf> = Design::EXAMPLES
        .iter()
        .map(|&design| {
            let d = temp(&format!("{}-fresh", design.name()));
            demo::create_design(&d, design).unwrap();
            d
        })
        .collect();
    for (design, fresh) in Design::EXAMPLES.into_iter().zip(fresh) {
        let committed = copy_example(design);
        let (a, b) = (bodies(&committed), bodies(&fresh));
        assert_eq!(a.len(), design.electrodes().len(), "{}", design.name());
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!((&x.0, &x.1), (&y.0, &y.1));
            // (OpenCASCADE bounding boxes of curved faces are sampled, slightly differently
            // each time: compare to 1e-4 of the body's size)
            let size = (0..3).map(|i| x.2[i + 3] - x.2[i]).fold(0.0, f64::max);
            assert!(x.2.iter().zip(&y.2).all(|(p, q)| (p - q).abs() < 1e-4 * size), "{}: {} {:?} vs {:?}", design.name(), x.0, x.2, y.2);
        }
        let project = |d: &Path| serde_json::from_str::<Value>(&std::fs::read_to_string(d.join("trapfields.json")).unwrap()).unwrap();
        assert_eq!(project(&committed), project(&fresh), "{}: regenerate examples/ with `trapfields demo`", design.name());
        let _ = std::fs::remove_dir_all(&committed);
        let _ = std::fs::remove_dir_all(&fresh);
    }
}

