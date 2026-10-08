//! The GEOS operations the osmarea compiler needs, through libgeos' C API.
//!
//! The library is opened at run time (`dlopen`), so building `teasi` needs
//! neither GEOS nor its headers; only `teasi osmarea` does.  Searched are
//! `$TEASI_GEOS`, then `libgeos_c.so.1` and `libgeos_c.so`.  For output that is
//! byte-identical to the Python compiler the same GEOS build has to be used --
//! shapely ships its own in `<venv>/lib/python3*/site-packages/shapely.libs/`.
//!
//! Only the handful of calls `tools/compile_osmarea.py` makes is bound, and
//! the wrappers keep shapely's semantics: `parts` is `shapely.get_parts`
//! (a polygon is its own single part), `rect` builds a ring exactly like
//! `shapely.box`, and the STRtree has shapely's node capacity so that queries
//! come back in the same order.

use std::cell::RefCell;
use std::ffi::{c_char, c_double, c_int, c_uint, c_void, CString};
use std::marker::PhantomData;
use std::sync::OnceLock;

use anyhow::{bail, Result};

type Ptr = *mut c_void;

extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> Ptr;
    fn dlsym(handle: Ptr, name: *const c_char) -> Ptr;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;

/// Declare the libgeos symbols we use; the field name is the symbol name.
macro_rules! api {
    ( $( fn $name:ident( $($arg:ty),* $(,)? ) $(-> $ret:ty)? ; )* ) => {
        #[allow(non_snake_case)]
        struct Api { $( $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )* }

        impl Api {
            unsafe fn load(h: Ptr) -> Result<Api> {
                Ok(Api { $( $name: {
                    let s = concat!(stringify!($name), "\0");
                    let p = dlsym(h, s.as_ptr() as *const c_char);
                    if p.is_null() {
                        bail!("libgeos has no symbol {}", &s[..s.len() - 1]);
                    }
                    std::mem::transmute(p)
                }, )* })
            }
        }
    };
}

type ErrHandler = Option<unsafe extern "C" fn(*const c_char, Ptr)>;
type TreeCb = Option<unsafe extern "C" fn(Ptr, Ptr)>;

api! {
    fn GEOS_init_r() -> Ptr;
    fn GEOSContext_setErrorMessageHandler_r(Ptr, ErrHandler, Ptr) -> Ptr;
    fn GEOSCoordSeq_create_r(Ptr, c_uint, c_uint) -> Ptr;
    fn GEOSCoordSeq_setXY_r(Ptr, Ptr, c_uint, c_double, c_double) -> c_int;
    fn GEOSCoordSeq_getXY_r(Ptr, Ptr, c_uint, *mut c_double, *mut c_double) -> c_int;
    fn GEOSCoordSeq_getSize_r(Ptr, Ptr, *mut c_uint) -> c_int;
    fn GEOSGeom_createLinearRing_r(Ptr, Ptr) -> Ptr;
    fn GEOSGeom_createPolygon_r(Ptr, Ptr, *mut Ptr, c_uint) -> Ptr;
    fn GEOSGeom_createEmptyPolygon_r(Ptr) -> Ptr;
    fn GEOSGeom_createCollection_r(Ptr, c_int, *mut Ptr, c_uint) -> Ptr;
    fn GEOSGeom_destroy_r(Ptr, Ptr);
    fn GEOSGeom_clone_r(Ptr, Ptr) -> Ptr;
    fn GEOSGeom_getCoordSeq_r(Ptr, Ptr) -> Ptr;
    fn GEOSGetExteriorRing_r(Ptr, Ptr) -> Ptr;
    fn GEOSGetNumInteriorRings_r(Ptr, Ptr) -> c_int;
    fn GEOSGetInteriorRingN_r(Ptr, Ptr, c_int) -> Ptr;
    fn GEOSGetNumGeometries_r(Ptr, Ptr) -> c_int;
    fn GEOSGetGeometryN_r(Ptr, Ptr, c_int) -> Ptr;
    fn GEOSGeomTypeId_r(Ptr, Ptr) -> c_int;
    fn GEOSisEmpty_r(Ptr, Ptr) -> c_char;
    fn GEOSisValid_r(Ptr, Ptr) -> c_char;
    fn GEOSMakeValid_r(Ptr, Ptr) -> Ptr;
    fn GEOSUnaryUnion_r(Ptr, Ptr) -> Ptr;
    fn GEOSUnion_r(Ptr, Ptr, Ptr) -> Ptr;
    fn GEOSSymDifference_r(Ptr, Ptr, Ptr) -> Ptr;
    fn GEOSDifference_r(Ptr, Ptr, Ptr) -> Ptr;
    fn GEOSIntersection_r(Ptr, Ptr, Ptr) -> Ptr;
    fn GEOSClipByRect_r(Ptr, Ptr, c_double, c_double, c_double, c_double) -> Ptr;
    fn GEOSTopologyPreserveSimplify_r(Ptr, Ptr, c_double) -> Ptr;
    fn GEOSContains_r(Ptr, Ptr, Ptr) -> c_char;
    fn GEOSIntersects_r(Ptr, Ptr, Ptr) -> c_char;
    fn GEOSArea_r(Ptr, Ptr, *mut c_double) -> c_int;
    fn GEOSGeom_getXMin_r(Ptr, Ptr, *mut c_double) -> c_int;
    fn GEOSGeom_getYMin_r(Ptr, Ptr, *mut c_double) -> c_int;
    fn GEOSGeom_getXMax_r(Ptr, Ptr, *mut c_double) -> c_int;
    fn GEOSGeom_getYMax_r(Ptr, Ptr, *mut c_double) -> c_int;
    fn GEOSPrepare_r(Ptr, Ptr) -> Ptr;
    fn GEOSPreparedGeom_destroy_r(Ptr, Ptr);
    fn GEOSPreparedIntersects_r(Ptr, Ptr, Ptr) -> c_char;
    fn GEOSSTRtree_create_r(Ptr, usize) -> Ptr;
    fn GEOSSTRtree_insert_r(Ptr, Ptr, Ptr, Ptr);
    fn GEOSSTRtree_query_r(Ptr, Ptr, Ptr, TreeCb, Ptr);
    fn GEOSSTRtree_destroy_r(Ptr, Ptr);
    fn GEOSversion() -> *const c_char;
}

const GEOS_POLYGON: c_int = 3;
const GEOS_MULTIPOLYGON: c_int = 6;
const GEOS_GEOMETRYCOLLECTION: c_int = 7;
/// shapely builds its STRtrees with this node capacity.
const NODE_CAPACITY: usize = 10;

fn api() -> Result<&'static Api> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    match API.get_or_init(|| unsafe {
        let mut names: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("TEASI_GEOS") {
            names.push(p);
        }
        names.push("libgeos_c.so.1".into());
        names.push("libgeos_c.so".into());
        let mut why: Vec<String> = Vec::new();
        for n in &names {
            let c = CString::new(n.as_str()).unwrap();
            let h = dlopen(c.as_ptr(), RTLD_NOW);
            if !h.is_null() {
                return Api::load(h).map_err(|e| e.to_string());
            }
            let e = dlerror();
            why.push(if e.is_null() {
                n.clone()
            } else {
                std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
            });
        }
        Err(format!(
            "libgeos not found; install it or point TEASI_GEOS at a libgeos_c.so\n  {}",
            why.join("\n  ")
        ))
    }) {
        Ok(a) => Ok(a),
        Err(e) => bail!("{}", e),
    }
}

thread_local! {
    static CTX: RefCell<Option<Ptr>> = const { RefCell::new(None) };
    static ERR: RefCell<String> = const { RefCell::new(String::new()) };
}

unsafe extern "C" fn on_error(msg: *const c_char, _: Ptr) {
    let s = std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned();
    ERR.with(|e| *e.borrow_mut() = s);
}

/// The calling thread's GEOS context; GEOS contexts are not shared.
fn ctx() -> Result<Ptr> {
    let a = api()?;
    CTX.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(p) = *c {
            return Ok(p);
        }
        let p = unsafe { (a.GEOS_init_r)() };
        if p.is_null() {
            bail!("GEOS_init_r failed");
        }
        unsafe { (a.GEOSContext_setErrorMessageHandler_r)(p, Some(on_error), std::ptr::null_mut()) };
        *c = Some(p);
        Ok(p)
    })
}

fn both() -> (&'static Api, Ptr) {
    (api().expect("libgeos"), ctx().expect("GEOS context"))
}

fn last_error() -> String {
    ERR.with(|e| e.borrow().clone())
}

/// The version string of the loaded library, e.g. "3.13.1-CAPI-1.19.2".
pub fn version() -> Result<String> {
    let a = api()?;
    Ok(unsafe { std::ffi::CStr::from_ptr((a.GEOSversion)()) }.to_string_lossy().into_owned())
}

/// Check that the library can be loaded, with a helpful error if not.
pub fn available() -> Result<()> {
    api().map(|_| ())
}

/// An owned GEOS geometry.
pub struct Geom(Ptr);

impl Drop for Geom {
    fn drop(&mut self) {
        if let (Ok(a), Ok(c)) = (api(), ctx()) {
            unsafe { (a.GEOSGeom_destroy_r)(c, self.0) }
        }
    }
}

fn owned(p: Ptr, what: &str) -> Result<Geom> {
    if p.is_null() {
        bail!("GEOS {} failed: {}", what, last_error());
    }
    Ok(Geom(p))
}

/// Coordinate sequence from points; GEOS takes it over on success.
fn seq(a: &Api, c: Ptr, pts: &[(f64, f64)]) -> Result<Ptr> {
    let s = unsafe { (a.GEOSCoordSeq_create_r)(c, pts.len() as c_uint, 2) };
    if s.is_null() {
        bail!("GEOS coord seq failed: {}", last_error());
    }
    for (i, &(x, y)) in pts.iter().enumerate() {
        unsafe { (a.GEOSCoordSeq_setXY_r)(c, s, i as c_uint, x, y) };
    }
    Ok(s)
}

/// Linear ring; an open point list is closed first, as shapely does it.
fn ring(a: &Api, c: Ptr, pts: &[(f64, f64)]) -> Result<Ptr> {
    let mut closed: &[(f64, f64)] = pts;
    let mut buf: Vec<(f64, f64)>;
    if pts.len() > 1 && pts[0] != pts[pts.len() - 1] {
        buf = pts.to_vec();
        buf.push(pts[0]);
        closed = &buf;
    }
    let s = seq(a, c, closed)?;
    let r = unsafe { (a.GEOSGeom_createLinearRing_r)(c, s) };
    if r.is_null() {
        bail!("GEOS linear ring failed: {}", last_error());
    }
    Ok(r)
}

/// Read a ring's points back.
fn ring_pts(a: &Api, c: Ptr, r: Ptr) -> Vec<(f64, f64)> {
    let s = unsafe { (a.GEOSGeom_getCoordSeq_r)(c, r) };
    let mut n: c_uint = 0;
    unsafe { (a.GEOSCoordSeq_getSize_r)(c, s, &mut n) };
    (0..n)
        .map(|i| {
            let (mut x, mut y) = (0.0, 0.0);
            unsafe { (a.GEOSCoordSeq_getXY_r)(c, s, i, &mut x, &mut y) };
            (x, y)
        })
        .collect()
}

impl Geom {
    /// Polygon from a shell and its holes, each a closed point list.
    pub fn polygon(shell: &[(f64, f64)], holes: &[Vec<(f64, f64)>]) -> Result<Geom> {
        let (a, c) = both();
        let sh = ring(a, c, shell)?;
        let mut hs: Vec<Ptr> = Vec::with_capacity(holes.len());
        for h in holes {
            hs.push(ring(a, c, h)?);
        }
        let p = unsafe {
            (a.GEOSGeom_createPolygon_r)(c, sh, hs.as_mut_ptr(), hs.len() as c_uint)
        };
        owned(p, "createPolygon")
    }

    /// The empty polygon, shapely's `Polygon()`.
    pub fn empty() -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSGeom_createEmptyPolygon_r)(c) }, "createEmptyPolygon")
    }

    /// A rectangle with exactly the ring `shapely.box` builds.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Geom> {
        Geom::polygon(&[(x1, y0), (x1, y1), (x0, y1), (x0, y0), (x1, y0)], &[])
    }

    pub fn clone_geom(&self) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSGeom_clone_r)(c, self.0) }, "clone")
    }

    pub fn type_id(&self) -> c_int {
        let (a, c) = both();
        unsafe { (a.GEOSGeomTypeId_r)(c, self.0) }
    }

    pub fn is_empty(&self) -> bool {
        let (a, c) = both();
        unsafe { (a.GEOSisEmpty_r)(c, self.0) == 1 }
    }

    pub fn is_valid(&self) -> bool {
        let (a, c) = both();
        unsafe { (a.GEOSisValid_r)(c, self.0) == 1 }
    }

    /// `shapely.make_valid` with its default (linework, collapses kept).
    pub fn make_valid(&self) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSMakeValid_r)(c, self.0) }, "makeValid")
    }

    /// The geometry itself if valid, otherwise repaired -- `valid()` in Python.
    pub fn valid(&self) -> Result<Geom> {
        if self.is_valid() {
            self.clone_geom()
        } else {
            self.make_valid()
        }
    }

    /// `shapely.get_parts`: the members of a collection, else the geometry itself.
    pub fn parts(&self) -> Result<Vec<Geom>> {
        let (a, c) = both();
        if self.type_id() < 4 {
            return Ok(vec![self.clone_geom()?]);
        }
        let n = unsafe { (a.GEOSGetNumGeometries_r)(c, self.0) };
        (0..n)
            .map(|i| {
                let g = unsafe { (a.GEOSGetGeometryN_r)(c, self.0, i) };
                owned(unsafe { (a.GEOSGeom_clone_r)(c, g) }, "clone")
            })
            .collect()
    }

    /// The non-empty polygon parts, `polygons()` in Python.
    pub fn polygons(&self) -> Result<Vec<Geom>> {
        let t = self.type_id();
        if t != GEOS_POLYGON && t != GEOS_MULTIPOLYGON && t != GEOS_GEOMETRYCOLLECTION {
            return Ok(Vec::new());
        }
        Ok(self
            .parts()?
            .into_iter()
            .filter(|p| p.type_id() == GEOS_POLYGON && !p.is_empty())
            .collect())
    }

    pub fn union(&self, o: &Geom) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSUnion_r)(c, self.0, o.0) }, "union")
    }

    pub fn difference(&self, o: &Geom) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSDifference_r)(c, self.0, o.0) }, "difference")
    }

    pub fn sym_difference(&self, o: &Geom) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSSymDifference_r)(c, self.0, o.0) }, "symDifference")
    }

    pub fn intersection(&self, o: &Geom) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSIntersection_r)(c, self.0, o.0) }, "intersection")
    }

    pub fn clip_by_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Geom> {
        let (a, c) = both();
        owned(unsafe { (a.GEOSClipByRect_r)(c, self.0, x0, y0, x1, y1) }, "clipByRect")
    }

    /// `simplify(tolerance, preserve_topology=True)`
    pub fn simplify(&self, tolerance: f64) -> Result<Geom> {
        let (a, c) = both();
        owned(
            unsafe { (a.GEOSTopologyPreserveSimplify_r)(c, self.0, tolerance) },
            "simplify",
        )
    }

    pub fn contains(&self, o: &Geom) -> bool {
        let (a, c) = both();
        unsafe { (a.GEOSContains_r)(c, self.0, o.0) == 1 }
    }

    pub fn intersects(&self, o: &Geom) -> bool {
        let (a, c) = both();
        unsafe { (a.GEOSIntersects_r)(c, self.0, o.0) == 1 }
    }

    pub fn area(&self) -> f64 {
        let (a, c) = both();
        let mut v = 0.0;
        unsafe { (a.GEOSArea_r)(c, self.0, &mut v) };
        v
    }

    /// (xmin, ymin, xmax, ymax), as `geom.bounds`.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (a, c) = both();
        let (mut x0, mut y0, mut x1, mut y1) = (0.0, 0.0, 0.0, 0.0);
        unsafe {
            (a.GEOSGeom_getXMin_r)(c, self.0, &mut x0);
            (a.GEOSGeom_getYMin_r)(c, self.0, &mut y0);
            (a.GEOSGeom_getXMax_r)(c, self.0, &mut x1);
            (a.GEOSGeom_getYMax_r)(c, self.0, &mut y1);
        }
        (x0, y0, x1, y1)
    }

    /// Points of a polygon's outer ring.
    pub fn exterior(&self) -> Vec<(f64, f64)> {
        let (a, c) = both();
        let r = unsafe { (a.GEOSGetExteriorRing_r)(c, self.0) };
        if r.is_null() {
            return Vec::new();
        }
        ring_pts(a, c, r)
    }

    /// Points of a polygon's holes.
    pub fn interiors(&self) -> Vec<Vec<(f64, f64)>> {
        let (a, c) = both();
        let n = unsafe { (a.GEOSGetNumInteriorRings_r)(c, self.0) };
        (0..n)
            .map(|i| {
                let r = unsafe { (a.GEOSGetInteriorRingN_r)(c, self.0, i) };
                ring_pts(a, c, r)
            })
            .collect()
    }
}

/// `shapely.union_all`: one unary union over the whole list.
pub fn union_all<'a, I: IntoIterator<Item = &'a Geom>>(gs: I) -> Result<Geom> {
    let (a, c) = both();
    let mut ps: Vec<Ptr> = Vec::new();
    for g in gs {
        ps.push(unsafe { (a.GEOSGeom_clone_r)(c, g.0) });
    }
    let coll = unsafe {
        (a.GEOSGeom_createCollection_r)(
            c,
            GEOS_GEOMETRYCOLLECTION,
            ps.as_mut_ptr(),
            ps.len() as c_uint,
        )
    };
    let coll = owned(coll, "createCollection")?;
    owned(unsafe { (a.GEOSUnaryUnion_r)(c, coll.0) }, "unaryUnion")
}

/// A prepared geometry for repeated intersects tests.
pub struct Prepared<'a>(Ptr, PhantomData<&'a Geom>);

impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        if let (Ok(a), Ok(c)) = (api(), ctx()) {
            unsafe { (a.GEOSPreparedGeom_destroy_r)(c, self.0) }
        }
    }
}

impl<'a> Prepared<'a> {
    pub fn new(g: &'a Geom) -> Result<Prepared<'a>> {
        let (a, c) = both();
        let p = unsafe { (a.GEOSPrepare_r)(c, g.0) };
        if p.is_null() {
            bail!("GEOS prepare failed: {}", last_error());
        }
        Ok(Prepared(p, PhantomData))
    }

    pub fn intersects(&self, o: &Geom) -> bool {
        let (a, c) = both();
        unsafe { (a.GEOSPreparedIntersects_r)(c, self.0, o.0) == 1 }
    }
}

/// A bounding box index over a geometry list, like `shapely.STRtree`.
pub struct Tree<'a>(Ptr, PhantomData<&'a [Geom]>);

impl Drop for Tree<'_> {
    fn drop(&mut self) {
        if let (Ok(a), Ok(c)) = (api(), ctx()) {
            unsafe { (a.GEOSSTRtree_destroy_r)(c, self.0) }
        }
    }
}

unsafe extern "C" fn collect(item: Ptr, data: Ptr) {
    (*(data as *mut Vec<usize>)).push(item as usize - 1);
}

impl<'a> Tree<'a> {
    pub fn new(gs: &'a [Geom]) -> Result<Tree<'a>> {
        let (a, c) = both();
        let t = unsafe { (a.GEOSSTRtree_create_r)(c, NODE_CAPACITY) };
        if t.is_null() {
            bail!("GEOS STRtree failed: {}", last_error());
        }
        for (i, g) in gs.iter().enumerate() {
            unsafe { (a.GEOSSTRtree_insert_r)(c, t, g.0, (i + 1) as Ptr) };
        }
        Ok(Tree(t, PhantomData))
    }

    /// Indices whose bounding box meets `g`, in the tree's own order.
    pub fn query(&self, g: &Geom) -> Vec<usize> {
        let (a, c) = both();
        let mut out: Vec<usize> = Vec::new();
        unsafe {
            (a.GEOSSTRtree_query_r)(
                c,
                self.0,
                g.0,
                Some(collect),
                &mut out as *mut Vec<usize> as Ptr,
            )
        };
        out
    }
}
