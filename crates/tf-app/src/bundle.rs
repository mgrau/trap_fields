//! Project bundles: the project file, its STEP file and the saved outputs (mesh, solution,
//! results) in one uncompressed POSIX tar archive, for download and upload.

use anyhow::{bail, ensure, Context, Result};
use std::path::{Component, Path};

/// Subdirectories of the project that hold saved outputs.
pub const OUTPUT_DIRS: [&str; 3] = ["mesh", "solve", "results"];
const BLOCK: usize = 512;

/// Files that belong in a bundle, relative to `dir`: the project file, `step_file`, and
/// everything directly inside the output directories.
pub fn project_files(dir: &Path, step_file: Option<&str>) -> Vec<String> {
    let mut v = vec![crate::project::PROJECT_FILE.to_string()];
    if let Some(f) = step_file {
        // a plain file name in the project folder, never a path elsewhere
        if allowed(f) && Path::new(f).components().count() == 1 && f != crate::project::PROJECT_FILE && dir.join(f).is_file() {
            v.push(f.to_string());
        }
    }
    for d in OUTPUT_DIRS {
        let mut names: Vec<String> = std::fs::read_dir(dir.join(d))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| format!("{d}/{}", e.file_name().to_string_lossy()))
            .collect();
        names.sort();
        v.extend(names);
    }
    v.retain(|f| dir.join(f).is_file());
    v
}

fn octal(field: &mut [u8], v: u64) {
    let s = format!("{:0w$o}\0", v, w = field.len() - 1);
    field.copy_from_slice(s.as_bytes());
}

/// A tar archive of `files` (paths relative to `dir`).
pub fn write_tar(dir: &Path, files: &[String]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for name in files {
        let data = std::fs::read(dir.join(name)).with_context(|| format!("reading {name}"))?;
        ensure!(name.len() < 100, "file name too long for the bundle: {name}");
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut h[100..108], 0o644);
        octal(&mut h[108..116], 0);
        octal(&mut h[116..124], 0);
        octal(&mut h[124..136], data.len() as u64);
        octal(&mut h[136..148], 0);
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        // checksum: header bytes with the checksum field taken as spaces
        h[148..156].copy_from_slice(b"        ");
        let sum: u64 = h.iter().map(|&b| b as u64).sum();
        let s = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(s.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(&data);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    Ok(out)
}

fn parse_octal(field: &[u8]) -> Result<u64> {
    let s: String = field.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).with_context(|| format!("bad number in tar header: {s:?}"))
}

/// Whether `name` is a path a bundle may contain: the project file or a STEP file at the
/// top level, or a plain file in one of the output directories.
pub fn allowed(name: &str) -> bool {
    let p = Path::new(name);
    let parts: Vec<_> = p.components().collect();
    if !parts.iter().all(|c| matches!(c, Component::Normal(_))) {
        return false;
    }
    let lower = name.to_lowercase();
    match parts.len() {
        1 => name == crate::project::PROJECT_FILE || lower.ends_with(".step") || lower.ends_with(".stp"),
        2 => OUTPUT_DIRS.iter().any(|d| Path::new(d) == Path::new(parts[0].as_os_str())),
        _ => false,
    }
}

/// Entries (name, contents) of a tar archive; regular files only, names checked with
/// [`allowed`].
pub fn read_tar(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut o = 0;
    while o + BLOCK <= data.len() {
        let h = &data[o..o + BLOCK];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let stored = parse_octal(&h[148..156])?;
        let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { b' ' as u64 } else { b as u64 }).sum();
        ensure!(stored == sum, "not a project bundle (bad tar checksum)");
        let mut name: String = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string();
        if &h[257..262] == b"ustar" && h[345] != 0 {
            let prefix = String::from_utf8_lossy(&h[345..500]).trim_end_matches('\0').to_string();
            name = format!("{prefix}/{name}");
        }
        let size = usize::try_from(parse_octal(&h[124..136])?).map_err(|_| anyhow::anyhow!("bundle entry too large"))?;
        let start = o + BLOCK;
        ensure!(size <= data.len() - start, "truncated bundle");
        let kind = h[156];
        if kind == b'0' || kind == 0 {
            let name = name.trim_start_matches("./").to_string();
            if !allowed(&name) {
                bail!("unexpected file in the bundle: {name}");
            }
            out.push((name, data[start..start + size].to_vec()));
        }
        o = start + size.div_ceil(BLOCK) * BLOCK; // ≤ data.len() + 511: no overflow
    }
    ensure!(out.iter().any(|(n, _)| n == crate::project::PROJECT_FILE), "not a project bundle (no {})", crate::project::PROJECT_FILE);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_checks() {
        let dir = std::env::temp_dir().join(format!("tf-bundle-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("mesh")).unwrap();
        std::fs::write(dir.join("trapfields.json"), b"{}").unwrap();
        std::fs::write(dir.join("trap.step"), b"ISO-10303-21;").unwrap();
        std::fs::write(dir.join("mesh/surface_mesh.bin"), vec![7u8; 1000]).unwrap();
        std::fs::write(dir.join("other.txt"), b"not bundled").unwrap();
        let files = project_files(&dir, Some("trap.step"));
        assert_eq!(files, ["trapfields.json", "trap.step", "mesh/surface_mesh.bin"]);
        let tar = write_tar(&dir, &files).unwrap();
        assert_eq!(tar.len() % 512, 0);
        let back = read_tar(&tar).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[2].1, vec![7u8; 1000]);
        // the archive is a valid tar for other tools too
        assert!(allowed("results/sweep.json") && allowed("x.STP"));
        assert!(!allowed("../evil.step") && !allowed("/etc/passwd") && !allowed("mesh/a/b") && !allowed("evil.sh"));
        let mut bad = tar.clone();
        bad[0] ^= 1;
        assert!(read_tar(&bad).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
