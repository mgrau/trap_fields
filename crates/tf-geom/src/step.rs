//! Lightweight STEP (ISO 10303-21) reader for the metadata gmsh/OpenCASCADE does not
//! expose: per-solid names (Fusion 360 writes body names on `MANIFOLD_SOLID_BREP`),
//! the file's length unit, and product names. It also collects each solid's B-rep
//! vertex coordinates so solids can be matched to gmsh volumes geometrically.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct StepSolid {
    pub entity: u64,
    pub name: String,
    /// Bounding box of the solid's B-rep vertices, in meters.
    pub vmin: [f64; 3],
    pub vmax: [f64; 3],
    pub n_vertices: usize,
}

#[derive(Debug, Clone)]
pub struct StepInfo {
    /// Meters per file length unit (1e-3 for mm).
    pub length_unit_m: f64,
    pub unit_name: String,
    pub solids: Vec<StepSolid>,
    pub products: Vec<String>,
    pub originating_system: String,
}

struct Entity<'a> {
    kind: &'a str,
    args: &'a str,
}

/// Split the DATA section into `#id = ...;` statements, honoring quoted strings and comments.
fn statements(text: &str) -> Vec<&str> {
    let start = text.find("DATA;").map(|i| i + 5).unwrap_or(0);
    let end = text[start..].find("ENDSEC;").map(|i| start + i).unwrap_or(text.len());
    let bytes = text.as_bytes();
    let (mut out, mut s, mut i, mut in_str) = (Vec::new(), start, start, false);
    while i < end {
        let c = bytes[i];
        if in_str {
            if c == b'\'' {
                in_str = false;
            }
        } else if c == b'\'' {
            in_str = true;
        } else if c == b'/' && i + 1 < end && bytes[i + 1] == b'*' {
            if let Some(j) = text[i + 2..end].find("*/") {
                let after = i + 2 + j + 2;
                if text[s..i].trim().is_empty() {
                    s = after; // comment before a statement: drop it
                }
                i = after;
                continue;
            }
        } else if c == b';' {
            out.push(text[s..i].trim());
            s = i + 1;
        }
        i += 1;
    }
    out
}

/// `#123` references in `args`, skipping quoted strings.
fn refs(args: &str) -> Vec<u64> {
    let b = args.as_bytes();
    let (mut out, mut i, mut in_str) = (Vec::new(), 0, false);
    while i < b.len() {
        let c = b[i];
        if c == b'\'' {
            in_str = !in_str;
        } else if !in_str && c == b'#' {
            let j = i + 1 + b[i + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
            if let Ok(id) = args[i + 1..j].parse() {
                out.push(id);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn first_string(args: &str) -> Option<String> {
    let s = args.find('\'')?;
    let mut out = String::new();
    let mut chars = args[s + 1..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                out.push('\'');
                chars.next();
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Numbers inside the last parenthesized group, e.g. `'',(1.,2.5,-3.E-2)`.
fn last_tuple(args: &str) -> Option<Vec<f64>> {
    let open = args.rfind('(')?;
    let close = open + args[open..].find(')')?;
    args[open + 1..close].split(',').map(|v| v.trim().parse::<f64>().ok()).collect()
}

fn length_unit(entities: &HashMap<u64, Entity>) -> (f64, String) {
    // in file order (entity ids), so a file with several unit contexts always gives the same one
    let mut ids: Vec<&u64> = entities.keys().collect();
    ids.sort_unstable();
    for e in ids.into_iter().map(|k| &entities[k]) {
        let body = if e.kind.is_empty() { e.args } else { continue };
        if !body.contains("LENGTH_UNIT") {
            continue;
        }
        if let Some(p) = body.find("SI_UNIT(") {
            let inner = &body[p + 8..];
            let prefix = inner.split(',').next().unwrap_or("").trim();
            let (f, n) = match prefix {
                ".MILLI." => (1e-3, "mm"),
                ".CENTI." => (1e-2, "cm"),
                ".MICRO." => (1e-6, "um"),
                ".NANO." => (1e-9, "nm"),
                ".KILO." => (1e3, "km"),
                ".DECI." => (1e-1, "dm"),
                _ => (1.0, "m"),
            };
            return (f, n.to_string());
        }
        if let Some(p) = body.find("CONVERSION_BASED_UNIT(") {
            let name = first_string(&body[p..]).unwrap_or_default().to_uppercase();
            let f = match name.as_str() {
                "INCH" => 0.0254,
                "FOOT" => 0.3048,
                "MIL" | "THOU" => 2.54e-5,
                "MICRON" => 1e-6,
                _ => continue,
            };
            return (f, name.to_lowercase());
        }
    }
    (1e-3, "mm (assumed)".to_string())
}

pub fn parse_step(text: &str) -> StepInfo {
    let mut entities: HashMap<u64, Entity> = HashMap::new();
    for st in statements(text) {
        let Some(rest) = st.strip_prefix('#') else { continue };
        let Some(eq) = rest.find('=') else { continue };
        let Ok(id) = rest[..eq].trim().parse::<u64>() else { continue };
        let body = rest[eq + 1..].trim();
        if body.starts_with('(') {
            entities.insert(id, Entity { kind: "", args: body });
        } else if let Some(p) = body.find('(') {
            let args = body[p + 1..].strip_suffix(')').unwrap_or(&body[p + 1..]);
            entities.insert(id, Entity { kind: body[..p].trim(), args });
        }
    }
    let (unit, unit_name) = length_unit(&entities);

    let mut ids: Vec<u64> = entities
        .iter()
        .filter(|(_, e)| matches!(e.kind, "MANIFOLD_SOLID_BREP" | "BREP_WITH_VOIDS" | "FACETED_BREP"))
        .map(|(&id, _)| id)
        .collect();
    ids.sort_unstable(); // file order

    let solids = ids
        .into_iter()
        .map(|id| {
            let e = &entities[&id];
            let name = first_string(e.args).unwrap_or_default();
            // DFS down the B-rep collecting VERTEX_POINT coordinates.
            let (mut lo, mut hi, mut n) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0);
            let mut seen = HashSet::new();
            let mut stack = vec![id];
            while let Some(i) = stack.pop() {
                if !seen.insert(i) {
                    continue;
                }
                let Some(ent) = entities.get(&i) else { continue };
                if ent.kind == "VERTEX_POINT" {
                    let p = refs(ent.args).into_iter().next().and_then(|r| entities.get(&r));
                    if let Some(xyz) = p.and_then(|p| last_tuple(p.args)) {
                        if xyz.len() == 3 {
                            for k in 0..3 {
                                lo[k] = lo[k].min(xyz[k] * unit);
                                hi[k] = hi[k].max(xyz[k] * unit);
                            }
                            n += 1;
                        }
                    }
                    continue;
                }
                stack.extend(refs(ent.args));
            }
            StepSolid { entity: id, name, vmin: lo, vmax: hi, n_vertices: n }
        })
        .collect();

    let mut products: Vec<(u64, String)> = entities
        .iter()
        .filter(|(_, e)| e.kind == "PRODUCT")
        .filter_map(|(&id, e)| {
            // PRODUCT('id','name',...) — prefer the name field if present
            let s: Vec<&str> = e.args.splitn(3, ',').collect();
            let name = s.get(1).and_then(|n| first_string(n)).filter(|n| !n.is_empty());
            name.or_else(|| first_string(e.args)).map(|n| (id, n))
        })
        .collect();
    products.sort();

    let header = &text[..text.find("DATA;").unwrap_or(0)];
    let originating_system = header
        .find("FILE_NAME")
        .map(|p| {
            // 6th string field of FILE_NAME
            let mut strs = Vec::new();
            let mut rest = &header[p..];
            while let Some(s) = rest.find('\'') {
                let Some(e) = rest[s + 1..].find('\'') else { break };
                strs.push(rest[s + 1..s + 1 + e].to_string());
                rest = &rest[s + e + 2..];
                if rest.trim_start().starts_with(')') && strs.len() >= 7 {
                    break;
                }
            }
            strs.get(5).cloned().unwrap_or_default()
        })
        .unwrap_or_default();

    StepInfo { length_unit_m: unit, unit_name, solids, products: products.into_iter().map(|p| p.1).collect(), originating_system }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = "ISO-10303-21;\nHEADER;\nFILE_NAME('a.step','t',(''),(''),'pp','Autodesk Translation Framework','');\nENDSEC;\nDATA;\n\
#1=MANIFOLD_SOLID_BREP('Blade ''A''',#2);\n#2=CLOSED_SHELL('',(#3,#4));\n\
#3=VERTEX_POINT('',#5);\n#4=VERTEX_POINT('',#6);\n#5=CARTESIAN_POINT('',(0.,1.5,-2.));\n\
#6=CARTESIAN_POINT('',(1.E1,-1.,3.));\n#7=( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
/* comment; with semicolon */\n#8=PRODUCT('id','My Trap',$,(#9));\nENDSEC;\nEND-ISO-10303-21;\n";

    #[test]
    fn parses_names_units_vertices() {
        let info = parse_step(MINI);
        assert_eq!(info.length_unit_m, 1e-3);
        assert_eq!(info.solids.len(), 1);
        let s = &info.solids[0];
        assert_eq!(s.name, "Blade 'A'");
        assert_eq!(s.n_vertices, 2);
        assert_eq!(s.vmin, [0.0, -1e-3, -2e-3]);
        assert_eq!(s.vmax, [1e-2, 1.5e-3, 3e-3]);
        assert_eq!(info.products, vec!["My Trap".to_string()]);
        assert_eq!(info.originating_system, "Autodesk Translation Framework");
    }
}
