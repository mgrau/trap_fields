//! Minimal safe wrapper around the gmsh C API (`gmshc.h`).
//!
//! gmsh keeps global state and is not thread-safe. Create exactly one [`Gmsh`]
//! and use it from a single thread (see `tf-server`'s worker thread).

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

pub type DimTag = (i32, i32);

#[derive(Debug, thiserror::Error)]
#[error("gmsh: {0}")]
pub struct GmshError(pub String);

pub type Result<T> = std::result::Result<T, GmshError>;

/// gmsh element type for 3-node triangles.
pub const ELEMENT_TRIANGLE: i32 = 2;
/// gmsh element type for 2-node lines.
pub const ELEMENT_LINE: i32 = 1;

#[allow(non_snake_case)]
mod ffi {
    use super::*;
    extern "C" {
        pub fn gmshInitialize(argc: c_int, argv: *mut *mut c_char, readConfigFiles: c_int, run: c_int, ierr: *mut c_int);
        pub fn gmshFinalize(ierr: *mut c_int);
        pub fn gmshClear(ierr: *mut c_int);
        pub fn gmshFree(p: *mut c_void);
        pub fn gmshOptionSetNumber(name: *const c_char, value: f64, ierr: *mut c_int);
        pub fn gmshOptionSetString(name: *const c_char, value: *const c_char, ierr: *mut c_int);
        pub fn gmshModelOccImportShapes(fileName: *const c_char, outDimTags: *mut *mut c_int, outDimTags_n: *mut usize, highestDimOnly: c_int, format: *const c_char, ierr: *mut c_int);
        pub fn gmshModelOccSynchronize(ierr: *mut c_int);
        pub fn gmshModelGetEntities(dimTags: *mut *mut c_int, dimTags_n: *mut usize, dim: c_int, ierr: *mut c_int);
        pub fn gmshModelGetBoundary(dimTags: *const c_int, dimTags_n: usize, outDimTags: *mut *mut c_int, outDimTags_n: *mut usize, combined: c_int, oriented: c_int, recursive: c_int, ierr: *mut c_int);
        pub fn gmshModelGetBoundingBox(dim: c_int, tag: c_int, xmin: *mut f64, ymin: *mut f64, zmin: *mut f64, xmax: *mut f64, ymax: *mut f64, zmax: *mut f64, ierr: *mut c_int);
        pub fn gmshModelOccGetMass(dim: c_int, tag: c_int, mass: *mut f64, ierr: *mut c_int);
        pub fn gmshModelOccGetCenterOfMass(dim: c_int, tag: c_int, x: *mut f64, y: *mut f64, z: *mut f64, ierr: *mut c_int);
        pub fn gmshModelGetType(dim: c_int, tag: c_int, entityType: *mut *mut c_char, ierr: *mut c_int);
        pub fn gmshModelGetAdjacencies(dim: c_int, tag: c_int, upward: *mut *mut c_int, upward_n: *mut usize, downward: *mut *mut c_int, downward_n: *mut usize, ierr: *mut c_int);
        pub fn gmshModelGetCurvature(dim: c_int, tag: c_int, parametricCoord: *const f64, parametricCoord_n: usize, curvatures: *mut *mut f64, curvatures_n: *mut usize, ierr: *mut c_int);
        pub fn gmshModelGetParametrizationBounds(dim: c_int, tag: c_int, min: *mut *mut f64, min_n: *mut usize, max: *mut *mut f64, max_n: *mut usize, ierr: *mut c_int);
        pub fn gmshModelMeshFieldSetNumbers(tag: c_int, option: *const c_char, values: *const f64, values_n: usize, ierr: *mut c_int);
        pub fn gmshModelGetValue(dim: c_int, tag: c_int, parametricCoord: *const f64, parametricCoord_n: usize, coord: *mut *mut f64, coord_n: *mut usize, ierr: *mut c_int);
        pub fn gmshModelSetVisibility(dimTags: *const c_int, dimTags_n: usize, value: c_int, recursive: c_int, ierr: *mut c_int);
        pub fn gmshModelMeshGenerate(dim: c_int, ierr: *mut c_int);
        pub fn gmshModelMeshClear(dimTags: *const c_int, dimTags_n: usize, ierr: *mut c_int);
        pub fn gmshModelMeshGetNodes(nodeTags: *mut *mut usize, nodeTags_n: *mut usize, coord: *mut *mut f64, coord_n: *mut usize, parametricCoord: *mut *mut f64, parametricCoord_n: *mut usize, dim: c_int, tag: c_int, includeBoundary: c_int, returnParametricCoord: c_int, ierr: *mut c_int);
        pub fn gmshModelMeshGetElementsByType(elementType: c_int, elementTags: *mut *mut usize, elementTags_n: *mut usize, nodeTags: *mut *mut usize, nodeTags_n: *mut usize, tag: c_int, task: usize, numTasks: usize, ierr: *mut c_int);
        pub fn gmshModelMeshFieldAdd(fieldType: *const c_char, tag: c_int, ierr: *mut c_int) -> c_int;
        pub fn gmshModelMeshFieldRemove(tag: c_int, ierr: *mut c_int);
        pub fn gmshModelMeshFieldSetNumber(tag: c_int, option: *const c_char, value: f64, ierr: *mut c_int);
        pub fn gmshModelMeshFieldSetString(tag: c_int, option: *const c_char, value: *const c_char, ierr: *mut c_int);
        pub fn gmshModelMeshFieldSetAsBackgroundMesh(tag: c_int, ierr: *mut c_int);
        pub fn gmshLoggerGetLastError(error: *mut *mut c_char, ierr: *mut c_int);
    }
}

static INITIALIZED: AtomicBool = AtomicBool::new(false);

fn last_error() -> String {
    let mut msg: *mut c_char = ptr::null_mut();
    let mut ierr = 0;
    unsafe {
        ffi::gmshLoggerGetLastError(&mut msg, &mut ierr);
        take_string(msg)
    }
}

fn check(ierr: c_int, what: &str) -> Result<()> {
    if ierr == 0 {
        Ok(())
    } else {
        let e = last_error();
        Err(GmshError(if e.is_empty() { format!("{what} failed (ierr={ierr})") } else { e }))
    }
}

/// Copy a gmsh-allocated array into a Vec and free the original.
unsafe fn take_vec<T: Copy>(p: *mut T, n: usize) -> Vec<T> {
    let v = if p.is_null() || n == 0 { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() };
    if !p.is_null() {
        ffi::gmshFree(p as *mut c_void);
    }
    v
}

unsafe fn take_string(p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    ffi::gmshFree(p as *mut c_void);
    s
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("string with interior NUL")
}

fn flatten(dim_tags: &[DimTag]) -> Vec<c_int> {
    dim_tags.iter().flat_map(|&(d, t)| [d, t]).collect()
}

fn pairs(v: Vec<c_int>) -> Vec<DimTag> {
    v.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Handle to the (global) gmsh library. Not `Send`/`Sync`: keep it on one thread.
pub struct Gmsh {
    _not_send: PhantomData<*const ()>,
}

impl Gmsh {
    pub fn initialize() -> Result<Gmsh> {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(GmshError("gmsh is already initialized".into()));
        }
        let mut ierr = 0;
        unsafe { ffi::gmshInitialize(0, ptr::null_mut(), 0, 0, &mut ierr) };
        if let Err(e) = check(ierr, "initialize") {
            INITIALIZED.store(false, Ordering::SeqCst);
            return Err(e);
        }
        let g = Gmsh { _not_send: PhantomData };
        g.set_number("General.Terminal", 0.0)?;
        Ok(g)
    }

    /// Remove all models, entities, meshes and fields.
    pub fn clear(&mut self) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshClear(&mut ierr) };
        check(ierr, "clear")
    }

    pub fn set_number(&self, name: &str, value: f64) -> Result<()> {
        let mut ierr = 0;
        let n = cstr(name);
        unsafe { ffi::gmshOptionSetNumber(n.as_ptr(), value, &mut ierr) };
        check(ierr, name)
    }

    pub fn set_string(&self, name: &str, value: &str) -> Result<()> {
        let mut ierr = 0;
        let (n, v) = (cstr(name), cstr(value));
        unsafe { ffi::gmshOptionSetString(n.as_ptr(), v.as_ptr(), &mut ierr) };
        check(ierr, name)
    }

    pub fn occ_import_shapes(&mut self, path: &Path) -> Result<Vec<DimTag>> {
        let p = cstr(&path.to_string_lossy());
        let fmt = cstr("");
        let (mut out, mut n, mut ierr) = (ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelOccImportShapes(p.as_ptr(), &mut out, &mut n, 1, fmt.as_ptr(), &mut ierr) };
        let v = unsafe { take_vec(out, n) };
        check(ierr, "importShapes")?;
        Ok(pairs(v))
    }

    pub fn occ_synchronize(&mut self) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshModelOccSynchronize(&mut ierr) };
        check(ierr, "synchronize")
    }

    /// Entities of dimension `dim` (-1 for all).
    pub fn entities(&self, dim: i32) -> Result<Vec<DimTag>> {
        let (mut out, mut n, mut ierr) = (ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelGetEntities(&mut out, &mut n, dim, &mut ierr) };
        let v = unsafe { take_vec(out, n) };
        check(ierr, "getEntities")?;
        Ok(pairs(v))
    }

    pub fn boundary(&self, dim_tags: &[DimTag], combined: bool, oriented: bool, recursive: bool) -> Result<Vec<DimTag>> {
        let input = flatten(dim_tags);
        let (mut out, mut n, mut ierr) = (ptr::null_mut(), 0usize, 0);
        unsafe {
            ffi::gmshModelGetBoundary(input.as_ptr(), input.len(), &mut out, &mut n, combined as c_int, oriented as c_int, recursive as c_int, &mut ierr)
        };
        let v = unsafe { take_vec(out, n) };
        check(ierr, "getBoundary")?;
        Ok(pairs(v))
    }

    /// (upward, downward) adjacent entity tags.
    pub fn adjacencies(&self, dim: i32, tag: i32) -> Result<(Vec<i32>, Vec<i32>)> {
        let (mut up, mut nu, mut down, mut nd, mut ierr) = (ptr::null_mut(), 0usize, ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelGetAdjacencies(dim, tag, &mut up, &mut nu, &mut down, &mut nd, &mut ierr) };
        let (u, d) = unsafe { (take_vec(up, nu), take_vec(down, nd)) };
        check(ierr, "getAdjacencies")?;
        Ok((u, d))
    }

    /// [xmin, ymin, zmin, xmax, ymax, zmax]
    pub fn bounding_box(&self, dim: i32, tag: i32) -> Result<[f64; 6]> {
        let mut b = [0.0f64; 6];
        let mut ierr = 0;
        let [x0, y0, z0, x1, y1, z1] = &mut b;
        unsafe { ffi::gmshModelGetBoundingBox(dim, tag, x0, y0, z0, x1, y1, z1, &mut ierr) };
        check(ierr, "getBoundingBox")?;
        Ok(b)
    }

    /// Length / area / volume of an OCC entity.
    pub fn occ_mass(&self, dim: i32, tag: i32) -> Result<f64> {
        let (mut m, mut ierr) = (0.0, 0);
        unsafe { ffi::gmshModelOccGetMass(dim, tag, &mut m, &mut ierr) };
        check(ierr, "getMass")?;
        Ok(m)
    }

    pub fn occ_center_of_mass(&self, dim: i32, tag: i32) -> Result<[f64; 3]> {
        let mut c = [0.0f64; 3];
        let mut ierr = 0;
        let [x, y, z] = &mut c;
        unsafe { ffi::gmshModelOccGetCenterOfMass(dim, tag, x, y, z, &mut ierr) };
        check(ierr, "getCenterOfMass")?;
        Ok(c)
    }

    pub fn entity_type(&self, dim: i32, tag: i32) -> Result<String> {
        let (mut s, mut ierr) = (ptr::null_mut(), 0);
        unsafe { ffi::gmshModelGetType(dim, tag, &mut s, &mut ierr) };
        let s = unsafe { take_string(s) };
        check(ierr, "getType")?;
        Ok(s)
    }

    /// Evaluate the parametrization of an entity. For points (dim 0) pass `&[]`.
    pub fn value(&self, dim: i32, tag: i32, param: &[f64]) -> Result<Vec<f64>> {
        let (mut out, mut n, mut ierr) = (ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelGetValue(dim, tag, param.as_ptr(), param.len(), &mut out, &mut n, &mut ierr) };
        let v = unsafe { take_vec(out, n) };
        check(ierr, "getValue")?;
        Ok(v)
    }

    /// Maximum curvature (1/m) of a curve or surface at parametric coordinates
    /// (u for curves, u,v pairs for surfaces).
    pub fn curvature(&self, dim: i32, tag: i32, param: &[f64]) -> Result<Vec<f64>> {
        let (mut out, mut n, mut ierr) = (ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelGetCurvature(dim, tag, param.as_ptr(), param.len(), &mut out, &mut n, &mut ierr) };
        let v = unsafe { take_vec(out, n) };
        check(ierr, "getCurvature")?;
        Ok(v)
    }

    /// (min, max) of the parametric coordinates of a curve or surface.
    pub fn parametrization_bounds(&self, dim: i32, tag: i32) -> Result<(Vec<f64>, Vec<f64>)> {
        let (mut lo, mut nlo, mut hi, mut nhi, mut ierr) = (ptr::null_mut(), 0usize, ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelGetParametrizationBounds(dim, tag, &mut lo, &mut nlo, &mut hi, &mut nhi, &mut ierr) };
        let (lo, hi) = unsafe { (take_vec(lo, nlo), take_vec(hi, nhi)) };
        check(ierr, "getParametrizationBounds")?;
        Ok((lo, hi))
    }

    pub fn set_visibility(&mut self, dim_tags: &[DimTag], visible: bool, recursive: bool) -> Result<()> {
        let input = flatten(dim_tags);
        let mut ierr = 0;
        unsafe { ffi::gmshModelSetVisibility(input.as_ptr(), input.len(), visible as c_int, recursive as c_int, &mut ierr) };
        check(ierr, "setVisibility")
    }

    pub fn mesh_generate(&mut self, dim: i32) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshGenerate(dim, &mut ierr) };
        check(ierr, "mesh.generate")
    }

    /// Clear the whole mesh.
    pub fn mesh_clear(&mut self) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshClear(ptr::null(), 0, &mut ierr) };
        check(ierr, "mesh.clear")
    }

    /// (node tags, flat xyz coordinates) of the mesh nodes classified on (dim, tag).
    pub fn mesh_nodes(&self, dim: i32, tag: i32, include_boundary: bool) -> Result<(Vec<usize>, Vec<f64>)> {
        let (mut tags, mut nt, mut xyz, mut nx, mut par, mut np, mut ierr) =
            (ptr::null_mut(), 0usize, ptr::null_mut(), 0usize, ptr::null_mut(), 0usize, 0);
        unsafe {
            ffi::gmshModelMeshGetNodes(&mut tags, &mut nt, &mut xyz, &mut nx, &mut par, &mut np, dim, tag, include_boundary as c_int, 0, &mut ierr)
        };
        let (t, x, _) = unsafe { (take_vec(tags, nt), take_vec(xyz, nx), take_vec(par, np)) };
        check(ierr, "mesh.getNodes")?;
        Ok((t, x))
    }

    /// (element tags, flat node tags) of all elements of `etype` on entity `tag`
    /// (`tag` = -1 for all entities).
    pub fn mesh_elements_by_type(&self, etype: i32, tag: i32) -> Result<(Vec<usize>, Vec<usize>)> {
        let (mut et, mut ne, mut nt, mut nn, mut ierr) = (ptr::null_mut(), 0usize, ptr::null_mut(), 0usize, 0);
        unsafe { ffi::gmshModelMeshGetElementsByType(etype, &mut et, &mut ne, &mut nt, &mut nn, tag, 0, 1, &mut ierr) };
        let (e, n) = unsafe { (take_vec(et, ne), take_vec(nt, nn)) };
        check(ierr, "mesh.getElementsByType")?;
        Ok((e, n))
    }

    pub fn field_add(&mut self, field_type: &str) -> Result<i32> {
        let t = cstr(field_type);
        let mut ierr = 0;
        let tag = unsafe { ffi::gmshModelMeshFieldAdd(t.as_ptr(), -1, &mut ierr) };
        check(ierr, "field.add")?;
        Ok(tag)
    }

    pub fn field_remove(&mut self, tag: i32) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshFieldRemove(tag, &mut ierr) };
        check(ierr, "field.remove")
    }

    pub fn field_set_number(&mut self, tag: i32, option: &str, value: f64) -> Result<()> {
        let o = cstr(option);
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshFieldSetNumber(tag, o.as_ptr(), value, &mut ierr) };
        check(ierr, option)
    }

    pub fn field_set_numbers(&mut self, tag: i32, option: &str, values: &[f64]) -> Result<()> {
        let o = cstr(option);
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshFieldSetNumbers(tag, o.as_ptr(), values.as_ptr(), values.len(), &mut ierr) };
        check(ierr, "field setNumbers")
    }

    pub fn field_set_string(&mut self, tag: i32, option: &str, value: &str) -> Result<()> {
        let (o, v) = (cstr(option), cstr(value));
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshFieldSetString(tag, o.as_ptr(), v.as_ptr(), &mut ierr) };
        check(ierr, option)
    }

    pub fn field_set_as_background(&mut self, tag: i32) -> Result<()> {
        let mut ierr = 0;
        unsafe { ffi::gmshModelMeshFieldSetAsBackgroundMesh(tag, &mut ierr) };
        check(ierr, "field.setAsBackgroundMesh")
    }
}

impl Drop for Gmsh {
    fn drop(&mut self) {
        let mut ierr = 0;
        unsafe { ffi::gmshFinalize(&mut ierr) };
        INITIALIZED.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_and_set_options() {
        let g = Gmsh::initialize().unwrap();
        g.set_number("Mesh.MeshSizeMax", 1.0).unwrap();
        assert!(g.set_number("No.Such.Option", 1.0).is_err());
        assert!(g.entities(-1).unwrap().is_empty());
    }
}
