#![allow(unsafe_code)]
//! C ABI for the core library; the contract lives in `include/osview.h`.
//!
//! Every entry point validates its pointers, lengths and handle state before
//! touching memory. A handle owns its inputs, the scored dataset and one
//! `last_error` string: failures record a message there, successes clear it.
//! The handle is not thread-safe; callers must serialise access to one handle.

use std::ffi::{c_char, CStr, CString};
use std::mem::size_of;
use std::slice;

use crate::{npy, ply, render, Camera, Cloud, Dataset, Matrix, View};

const OK: i32 = 0;
const ERR_ARG: i32 = 1;
const ERR_STATE: i32 = 2;

/// Maximum sane render dimension accepted by the camera validator.
const MAX_DIM: u32 = 8192;
/// Maximum splat radius accepted by the camera validator.
const MAX_RADIUS: i32 = 64;

/// Opaque C handle (`typedef struct Osview Osview`).
pub struct Osview {
    cloud: Option<Cloud>,
    features: Option<Matrix>,
    /// Copy of the query, L2-normalized like feature rows.
    query: Option<Vec<f32>>,
    /// Scored dataset; while set, it owns the cloud/features/query above.
    dataset: Option<Dataset>,
    last_error: CString,
}

/// Mirror of `OsviewCamera` in `include/osview.h` (44 bytes, C layout).
#[repr(C)]
pub struct OsviewCamera {
    pub view: u32,
    pub width: u32,
    pub height: u32,
    pub radius: i32,
    pub range_min: f32,
    pub range_max: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan_x: i32,
    pub pan_y: i32,
}

/// Mirror of `OsviewMatch` in `include/osview.h` (20 bytes, C layout).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OsviewMatch {
    pub index: u32,
    pub score: f32,
    pub p: [f32; 3],
}

fn to_cstring(msg: &str) -> CString {
    // Paths arrive as NUL-terminated C strings, so embedded NUL is impossible;
    // fall back to an empty (cleared) message only if a caller smuggles one in.
    CString::new(msg).unwrap_or_default()
}

impl Osview {
    fn ok(&mut self) -> i32 {
        self.last_error = CString::default();
        OK
    }

    fn fail(&mut self, code: i32, msg: impl AsRef<str>) -> i32 {
        self.last_error = to_cstring(msg.as_ref());
        code
    }

    /// Drop the scored dataset, moving its inputs back into the handle.
    fn invalidate(&mut self) {
        if let Some(ds) = self.dataset.take() {
            self.cloud = Some(ds.cloud);
            self.features = Some(ds.features);
            self.query = Some(ds.query);
        }
    }

    fn point_count(&self) -> usize {
        match &self.dataset {
            Some(ds) => ds.cloud.xyz.len(),
            None => self.cloud.as_ref().map_or(0, |c| c.xyz.len()),
        }
    }

    fn feature_dim(&self) -> usize {
        match &self.dataset {
            Some(ds) => ds.features.dim,
            None => self.features.as_ref().map_or(0, |f| f.dim),
        }
    }
}

/// Borrow a handle for the duration of one entry-point call.
///
/// # Safety
/// `h` must be null, or a pointer returned by [`osview_new`] that has not been
/// freed. The caller must not keep another Rust reference into it alive.
unsafe fn handle_mut<'a>(h: *const Osview) -> Option<&'a mut Osview> {
    unsafe { (h as *mut Osview).as_mut() }
}

/// Borrow a NUL-terminated argument.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated byte string valid for reads.
unsafe fn c_str_arg<'a>(p: *const c_char) -> Option<&'a CStr> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) })
    }
}

/// L2-normalize one vector, mirroring `npy::normalize_rows`: a zero (or
/// non-finite) norm collapses the vector to zeros rather than producing NaNs.
fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 1e-12 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    } else {
        v.fill(0.0);
    }
}

static VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

const NULL_ERROR: &[u8] = b"null handle\0";

/// `const char *osview_version(void)`.
#[no_mangle]
pub extern "C" fn osview_version() -> *const c_char {
    VERSION.as_ptr().cast()
}

/// `Osview *osview_new(void)`.
#[no_mangle]
pub extern "C" fn osview_new() -> *mut Osview {
    Box::into_raw(Box::new(Osview {
        cloud: None,
        features: None,
        query: None,
        dataset: None,
        last_error: CString::default(),
    }))
}

/// `void osview_free(Osview *h)`; null-safe.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle from [`osview_new`] that has not been freed.
pub unsafe extern "C" fn osview_free(h: *mut Osview) {
    if h.is_null() {
        return;
    }
    // SAFETY: `h` came from `osview_new` and is freed exactly once; the caller
    // must not use it (or its last_error pointer) afterwards.
    drop(unsafe { Box::from_raw(h) });
}

/// `int osview_load_ply(Osview *h, const char *path)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle; `path` must be NULL or a NUL-terminated,
/// readable string.
pub unsafe extern "C" fn osview_load_ply(h: *mut Osview, path: *const c_char) -> i32 {
    // SAFETY: the caller passes a live handle (or null) and nothing else
    // borrows it for the duration of this call.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return ERR_ARG;
    };
    // SAFETY: a non-null path is NUL-terminated and readable.
    let Some(path) = (unsafe { c_str_arg(path) }) else {
        return h.fail(ERR_ARG, "osview_load_ply: path is NULL");
    };
    let Ok(path) = path.to_str() else {
        return h.fail(ERR_STATE, "osview_load_ply: path is not valid UTF-8");
    };
    match ply::read(path) {
        Ok(cloud) => {
            h.invalidate();
            h.cloud = Some(cloud);
            h.ok()
        }
        Err(e) => h.fail(ERR_STATE, format!("{path}: {e}")),
    }
}

/// `int osview_load_features(Osview *h, const char *path)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle; `path` must be NULL or a NUL-terminated,
/// readable string.
pub unsafe extern "C" fn osview_load_features(h: *mut Osview, path: *const c_char) -> i32 {
    // SAFETY: same contract as osview_load_ply.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return ERR_ARG;
    };
    // SAFETY: a non-null path is NUL-terminated and readable.
    let Some(path) = (unsafe { c_str_arg(path) }) else {
        return h.fail(ERR_ARG, "osview_load_features: path is NULL");
    };
    let Ok(path) = path.to_str() else {
        return h.fail(ERR_STATE, "osview_load_features: path is not valid UTF-8");
    };
    match npy::read(path) {
        Ok(features) => {
            h.invalidate();
            h.features = Some(features);
            h.ok()
        }
        Err(e) => h.fail(ERR_STATE, format!("{path}: {e}")),
    }
}

/// `int osview_set_query(Osview *h, const float *q, size_t dim)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle; `q` must be NULL or point to at least
/// `dim` readable floats.
pub unsafe extern "C" fn osview_set_query(h: *mut Osview, q: *const f32, dim: usize) -> i32 {
    // SAFETY: same contract as osview_load_ply.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return ERR_ARG;
    };
    if q.is_null() {
        return h.fail(ERR_ARG, "osview_set_query: query is NULL");
    }
    if dim == 0 {
        return h.fail(ERR_ARG, "osview_set_query: dim must be > 0");
    }
    if dim > isize::MAX as usize / size_of::<f32>() {
        return h.fail(ERR_ARG, "osview_set_query: dim overflows the address space");
    }
    // SAFETY: the caller promises `q` points to at least `dim` readable floats;
    // `dim` was bounds-checked against isize::MAX above.
    let src = unsafe { slice::from_raw_parts(q, dim) };
    let mut query = src.to_vec();
    normalize(&mut query);
    h.invalidate();
    h.query = Some(query);
    h.ok()
}

/// `int osview_score(Osview *h)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle from [`osview_new`].
pub unsafe extern "C" fn osview_score(h: *mut Osview) -> i32 {
    // SAFETY: same contract as osview_load_ply.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return ERR_ARG;
    };
    if h.dataset.is_some() {
        return h.ok();
    }

    // Validate before consuming the inputs so a failure leaves them reusable.
    // The checks mirror `Dataset::from_parts` exactly.
    let bad = match (&h.cloud, &h.features, &h.query) {
        (Some(cloud), Some(features), Some(query)) => {
            if features.rows != cloud.xyz.len() {
                Some(format!(
                    "feature rows ({}) != points ({}) — osview pairs the i-th feature row \
                     with the i-th vertex",
                    features.rows,
                    cloud.xyz.len()
                ))
            } else if query.len() != features.dim {
                Some(format!(
                    "query dim ({}) != feature dim ({}) — same CLIP text tower must have \
                     produced both",
                    query.len(),
                    features.dim
                ))
            } else if query.iter().all(|v| v.abs() < 1e-9) {
                Some(
                    "query vector is all zeros — refusing to emit an all-zero/one-color \
                     image; pass a real text embedding"
                        .to_string(),
                )
            } else {
                None
            }
        }
        (None, _, _) => Some("cannot score: point cloud not loaded".to_string()),
        (_, None, _) => Some("cannot score: feature matrix not loaded".to_string()),
        (_, _, None) => Some("cannot score: query not set".to_string()),
    };
    if let Some(msg) = bad {
        return h.fail(ERR_STATE, msg);
    }

    let (cloud, features, query) = match (h.cloud.take(), h.features.take(), h.query.take()) {
        (Some(c), Some(f), Some(q)) => (c, f, q),
        // Unreachable after the checks above, but no panic across the ABI.
        _ => return h.fail(ERR_STATE, "cannot score: inputs unavailable"),
    };
    let dim = features.dim;
    let query = Matrix {
        rows: 1,
        dim,
        data: query,
    };
    match Dataset::from_parts(cloud, features, query) {
        Ok(ds) => {
            h.dataset = Some(ds);
            h.ok()
        }
        // Validation above makes this unreachable; inputs are consumed here.
        Err(e) => h.fail(ERR_STATE, format!("cannot score: {e}")),
    }
}

/// `int osview_render(const Osview *h, const OsviewCamera *cam, uint8_t *rgb, size_t rgb_len)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle; `cam` must be NULL or readable; `rgb`
/// must be NULL or point to at least `rgb_len` writable bytes.
pub unsafe extern "C" fn osview_render(
    h: *const Osview,
    cam: *const OsviewCamera,
    rgb: *mut u8,
    rgb_len: usize,
) -> i32 {
    // SAFETY: the handle is live for this call; `const` only reflects that the
    // caller does not mutate through the pointer, while `last_error` is still
    // owned and written by the handle.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return ERR_ARG;
    };
    // SAFETY: a non-null camera points to a readable OsviewCamera.
    let Some(cam) = (unsafe { cam.as_ref() }) else {
        return h.fail(ERR_ARG, "osview_render: camera is NULL");
    };
    let cam = match camera_from(cam) {
        Ok(cam) => cam,
        Err(e) => return h.fail(ERR_ARG, e),
    };
    let need = cam
        .width
        .checked_mul(cam.height)
        .and_then(|n| n.checked_mul(3));
    let Some(need) = need else {
        return h.fail(ERR_ARG, "osview_render: width*height*3 overflows usize");
    };
    if rgb.is_null() {
        return h.fail(ERR_ARG, "osview_render: rgb is NULL");
    }
    if rgb_len < need {
        return h.fail(
            ERR_ARG,
            format!("osview_render: rgb_len {rgb_len} < {need} (width*height*3)"),
        );
    }

    let empty: &[f32] = &[];
    let (cloud, scores) = match h.dataset.as_ref() {
        Some(ds) => (&ds.cloud, ds.scores.as_slice()),
        None => match h.cloud.as_ref() {
            Some(cloud) => (cloud, empty),
            None => return h.fail(ERR_STATE, "cannot render: point cloud not loaded"),
        },
    };
    let img = render::render(cloud, scores, &cam);
    // SAFETY: `rgb` is non-null and points to `rgb_len >= need` writable bytes;
    // `img` is exactly `need` bytes because render builds width*height*3.
    let out = unsafe { slice::from_raw_parts_mut(rgb, need) };
    out.copy_from_slice(&img);
    h.ok()
}

/// Validate a C camera and convert it into the core `Camera`.
fn camera_from(cam: &OsviewCamera) -> Result<Camera, String> {
    let view = match cam.view {
        0 => View::XY,
        1 => View::XZ,
        2 => View::YZ,
        v => return Err(format!("camera.view {v} is not 0 (xy), 1 (xz) or 2 (yz)")),
    };
    if cam.width == 0 || cam.width > MAX_DIM || cam.height == 0 || cam.height > MAX_DIM {
        return Err(format!(
            "camera width/height must be in 1..={MAX_DIM}, got {}x{}",
            cam.width, cam.height
        ));
    }
    if !(0..=MAX_RADIUS).contains(&cam.radius) {
        return Err(format!(
            "camera.radius must be in 0..={MAX_RADIUS}, got {}",
            cam.radius
        ));
    }
    if !(cam.zoom.is_finite() && cam.zoom > 0.0) {
        return Err(format!(
            "camera.zoom must be finite and > 0, got {}",
            cam.zoom
        ));
    }
    if !(cam.yaw.is_finite() && cam.pitch.is_finite()) {
        return Err("camera.yaw and camera.pitch must be finite".to_string());
    }
    if !(cam.range_min.is_finite() && cam.range_max.is_finite()) || cam.range_max <= cam.range_min {
        return Err(format!(
            "camera.range_max ({}) must be > range_min ({})",
            cam.range_max, cam.range_min
        ));
    }
    Ok(Camera {
        view,
        width: cam.width as usize,
        height: cam.height as usize,
        radius: cam.radius,
        range_min: cam.range_min,
        range_max: cam.range_max,
        yaw: cam.yaw,
        pitch: cam.pitch,
        zoom: cam.zoom,
        pan_x: cam.pan_x,
        pan_y: cam.pan_y,
    })
}

/// `int osview_top_k(const Osview *h, size_t k, OsviewMatch *out, size_t out_len)`.
///
/// Returns the number of matches written (<= k, out_len and points), or a
/// negative value: -1 for a null/invalid argument, -2 for unscored state.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle; `out` must be NULL or point to at least
/// `out_len` writable `OsviewMatch` slots.
pub unsafe extern "C" fn osview_top_k(
    h: *const Osview,
    k: usize,
    out: *mut OsviewMatch,
    out_len: usize,
) -> i32 {
    // SAFETY: same contract as osview_render.
    let Some(h) = (unsafe { handle_mut(h) }) else {
        return -1;
    };
    if out_len > 0 && out.is_null() {
        return h.fail(-1, "osview_top_k: out is NULL");
    }
    if out_len > isize::MAX as usize / size_of::<OsviewMatch>() {
        return h.fail(-1, "osview_top_k: out_len overflows isize::MAX");
    }
    let Some(ds) = h.dataset.as_ref() else {
        return h.fail(-2, "cannot rank: call osview_score first");
    };
    // `OsviewMatch::index` is u32 and the return value is int.
    if ds.scores.len() > i32::MAX as usize {
        return h.fail(-2, "cannot rank: point count exceeds i32::MAX");
    }
    let n = k.min(out_len).min(ds.scores.len());
    if n == 0 {
        return h.ok();
    }
    let matches = ds.top_k(n);
    // SAFETY: `out` is non-null because n > 0 implies out_len > 0, and the
    // caller promises out_len writable OsviewMatch slots (bounds-checked above).
    let slots = unsafe { slice::from_raw_parts_mut(out, n) };
    for (slot, m) in slots.iter_mut().zip(matches) {
        *slot = OsviewMatch {
            index: m.index as u32, // scores.len() <= i32::MAX checked above
            score: m.score,
            p: m.p,
        };
    }
    h.last_error = CString::default();
    n as i32
}

/// `size_t osview_points(const Osview *h)`; 0 for null or no point cloud.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle from [`osview_new`].
pub unsafe extern "C" fn osview_points(h: *const Osview) -> usize {
    // SAFETY: `h` is null or a live handle; only immutable access happens here.
    match unsafe { h.as_ref() } {
        Some(h) => h.point_count(),
        None => 0,
    }
}

/// `size_t osview_dim(const Osview *h)`; 0 for null or no feature matrix.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle from [`osview_new`].
pub unsafe extern "C" fn osview_dim(h: *const Osview) -> usize {
    // SAFETY: `h` is null or a live handle; only immutable access happens here.
    match unsafe { h.as_ref() } {
        Some(h) => h.feature_dim(),
        None => 0,
    }
}

/// `const char *osview_last_error(const Osview *h)`.
#[no_mangle]
/// # Safety
/// `h` must be NULL or a live handle from [`osview_new`].
pub unsafe extern "C" fn osview_last_error(h: *const Osview) -> *const c_char {
    // SAFETY: `h` is null or a live handle; only immutable access happens here.
    match unsafe { h.as_ref() } {
        Some(h) => h.last_error.as_ptr(),
        None => NULL_ERROR.as_ptr().cast(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    const PLY: &str = "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\n\
property float y\nproperty float z\nend_header\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n";

    fn write_npy(path: &Path, rows: usize, dim: usize, data: &[f32]) {
        let header =
            format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({rows}, {dim}), }}");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x93, b'N', b'U', b'M', b'P', b'Y', 1, 0]);
        bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
        bytes.extend_from_slice(header.as_bytes());
        for v in data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }

    struct Fixture {
        ply: PathBuf,
        feats: PathBuf,
        query: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for p in [&self.ply, &self.feats, &self.query] {
                let _ = fs::remove_file(p);
            }
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let dir = std::env::temp_dir();
        let stem = format!("osview-ffi-{}-{tag}", std::process::id());
        let f = Fixture {
            ply: dir.join(format!("{stem}.ply")),
            feats: dir.join(format!("{stem}-feats.npy")),
            query: dir.join(format!("{stem}-query.npy")),
        };
        fs::write(&f.ply, PLY).unwrap();
        write_npy(&f.feats, 4, 2, &[1.0, 0.0, 0.6, 0.8, 0.0, 1.0, 0.8, 0.6]);
        write_npy(&f.query, 1, 2, &[1.0, 0.0]);
        f
    }

    fn cpath(p: &Path) -> CString {
        CString::new(p.to_str().unwrap()).unwrap()
    }

    fn camera(width: u32, height: u32) -> OsviewCamera {
        OsviewCamera {
            view: 0,
            width,
            height,
            radius: 1,
            range_min: 0.05,
            range_max: 0.10,
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
        }
    }

    fn last_error(h: *const Osview) -> String {
        // SAFETY: the handle outlives this read and the pointer is valid until
        // the next call that mutates the handle.
        unsafe { CStr::from_ptr(osview_last_error(h)) }
            .to_string_lossy()
            .into_owned()
    }

    fn painted(rgb: &[u8]) -> usize {
        rgb.chunks_exact(3).filter(|px| *px != [24, 24, 28]).count()
    }

    #[test]
    fn lifecycle_and_null_handling() {
        let version = unsafe { CStr::from_ptr(osview_version()) };
        assert_eq!(version.to_str().unwrap(), env!("CARGO_PKG_VERSION"));

        // Every entry point tolerates a null handle / null argument.
        let mut out = [OsviewMatch {
            index: 0,
            score: 0.0,
            p: [0.0; 3],
        }; 2];
        assert_eq!(unsafe { osview_points(std::ptr::null()) }, 0);
        assert_eq!(unsafe { osview_dim(std::ptr::null()) }, 0);
        assert_eq!(
            unsafe { CStr::from_ptr(osview_last_error(std::ptr::null())) }
                .to_str()
                .unwrap(),
            "null handle"
        );
        assert_eq!(
            unsafe { osview_load_ply(std::ptr::null_mut(), std::ptr::null()) },
            1
        );
        assert_eq!(
            unsafe { osview_load_features(std::ptr::null_mut(), std::ptr::null()) },
            1
        );
        assert_eq!(
            unsafe { osview_set_query(std::ptr::null_mut(), std::ptr::null(), 1) },
            1
        );
        assert_eq!(unsafe { osview_score(std::ptr::null_mut()) }, 1);
        assert_eq!(
            unsafe { osview_render(std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), 0) },
            1
        );
        assert_eq!(
            unsafe { osview_top_k(std::ptr::null(), 1, out.as_mut_ptr(), out.len()) },
            -1
        );
        unsafe { osview_free(std::ptr::null_mut()) };

        // Fresh handle: no state, every operation reports its own problem.
        let h = osview_new();
        assert!(!h.is_null());
        assert_eq!(unsafe { osview_points(h) }, 0);
        assert_eq!(unsafe { osview_dim(h) }, 0);
        assert_eq!(last_error(h), "");
        assert_eq!(unsafe { osview_score(h) }, 2);
        assert!(last_error(h).contains("point cloud not loaded"));
        assert_eq!(
            unsafe { osview_top_k(h, 1, out.as_mut_ptr(), out.len()) },
            -2
        );
        assert!(last_error(h).contains("osview_score"));
        let cam = camera(16, 16);
        let mut rgb = vec![0u8; 16 * 16 * 3];
        assert_eq!(
            unsafe { osview_render(h, &cam, rgb.as_mut_ptr(), rgb.len()) },
            2
        );
        assert!(last_error(h).contains("point cloud not loaded"));
        assert_eq!(unsafe { osview_set_query(h, std::ptr::null(), 2) }, 1);
        assert!(last_error(h).contains("NULL"));
        let q = [1.0f32, 0.0];
        assert_eq!(unsafe { osview_set_query(h, q.as_ptr(), 0) }, 1);
        assert!(last_error(h).contains("dim"));
        unsafe { osview_free(h) };
    }

    #[test]
    fn load_score_render_topk() {
        let f = fixture("happy");
        let h = osview_new();
        let (ply, feats) = (cpath(&f.ply), cpath(&f.feats));

        assert_eq!(unsafe { osview_load_ply(h, ply.as_ptr()) }, 0);
        assert_eq!(last_error(h), "");
        assert_eq!(unsafe { osview_points(h) }, 4);
        assert_eq!(unsafe { osview_dim(h) }, 0);

        assert_eq!(unsafe { osview_load_features(h, feats.as_ptr()) }, 0);
        assert_eq!(unsafe { osview_dim(h) }, 2);
        let q = [1.0f32, 0.0];
        assert_eq!(unsafe { osview_set_query(h, q.as_ptr(), 2) }, 0);
        assert_eq!(unsafe { osview_score(h) }, 0);
        assert_eq!(last_error(h), "");

        let cam = camera(64, 48);
        let mut rgb = vec![0u8; 64 * 48 * 3];
        assert_eq!(
            unsafe { osview_render(h, &cam, rgb.as_mut_ptr(), rgb.len()) },
            0
        );
        assert!(painted(&rgb) > 0, "render painted no foreground pixels");

        let mut out = [OsviewMatch {
            index: 0,
            score: 0.0,
            p: [0.0; 3],
        }; 3];
        assert_eq!(
            unsafe { osview_top_k(h, 5, out.as_mut_ptr(), out.len()) },
            3
        );
        assert_eq!(unsafe { osview_top_k(h, 10, out.as_mut_ptr(), 0) }, 0);
        let idx: Vec<u32> = out.iter().map(|m| m.index).collect();
        assert_eq!(idx, vec![0, 3, 1]);
        assert!((out[0].score - 1.0).abs() < 1e-6);
        assert!((out[1].score - 0.8).abs() < 1e-6);
        assert_eq!(out[0].p, [0.0, 0.0, 0.0]);
        assert_eq!(out[1].p, [0.0, 0.0, 1.0]);

        // k limits the count too, and out_len is the hard cap.
        assert_eq!(
            unsafe { osview_top_k(h, 2, out.as_mut_ptr(), out.len()) },
            2
        );
        let mut one = [OsviewMatch {
            index: 9,
            score: 0.0,
            p: [0.0; 3],
        }; 1];
        assert_eq!(unsafe { osview_top_k(h, 5, one.as_mut_ptr(), 1) }, 1);
        assert_eq!(one[0].index, 0);

        // A new query invalidates the scored dataset until rescored.
        let q2 = [0.0f32, 1.0];
        assert_eq!(unsafe { osview_set_query(h, q2.as_ptr(), 2) }, 0);
        assert_eq!(unsafe { osview_top_k(h, 1, one.as_mut_ptr(), 1) }, -2);
        assert_eq!(unsafe { osview_score(h) }, 0);
        assert_eq!(unsafe { osview_top_k(h, 1, one.as_mut_ptr(), 1) }, 1);
        assert_eq!(one[0].index, 2);

        // Reloading features also invalidates; the old scores must not survive.
        assert_eq!(unsafe { osview_load_features(h, feats.as_ptr()) }, 0);
        assert_eq!(unsafe { osview_top_k(h, 1, one.as_mut_ptr(), 1) }, -2);
        assert_eq!(unsafe { osview_score(h) }, 0);

        unsafe { osview_free(h) };
    }

    #[test]
    fn camera_validation_sets_and_clears_error() {
        let f = fixture("camera");
        let h = osview_new();
        let (ply, feats) = (cpath(&f.ply), cpath(&f.feats));
        let q = [1.0f32, 0.0];
        unsafe {
            assert_eq!(osview_load_ply(h, ply.as_ptr()), 0);
            assert_eq!(osview_load_features(h, feats.as_ptr()), 0);
            assert_eq!(osview_set_query(h, q.as_ptr(), 2), 0);
            assert_eq!(osview_score(h), 0);
        }
        let mut rgb = vec![0u8; 32 * 24 * 3];

        // Short buffer: failure records a message; a valid call clears it.
        let cam = camera(32, 24);
        assert_eq!(
            unsafe { osview_render(h, &cam, rgb.as_mut_ptr(), rgb.len() - 1) },
            1
        );
        assert!(last_error(h).contains("rgb_len"));
        assert_eq!(
            unsafe { osview_render(h, &cam, rgb.as_mut_ptr(), rgb.len()) },
            0
        );
        assert_eq!(last_error(h), "");

        let mut bad = camera(32, 24);
        bad.view = 3;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        assert!(last_error(h).contains("view"));
        bad = camera(32, 24);
        bad.width = 0;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(8193, 24);
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), 8193 * 24 * 3) },
            1
        );
        bad = camera(32, 24);
        bad.radius = 65;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(32, 24);
        bad.radius = -1;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(32, 24);
        bad.zoom = 0.0;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(32, 24);
        bad.zoom = f32::NAN;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(32, 24);
        bad.range_max = bad.range_min;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        bad = camera(32, 24);
        bad.yaw = f32::INFINITY;
        assert_eq!(
            unsafe { osview_render(h, &bad, rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        assert_eq!(
            unsafe { osview_render(h, std::ptr::null(), rgb.as_mut_ptr(), rgb.len()) },
            1
        );
        assert_eq!(
            unsafe { osview_render(h, &cam, std::ptr::null_mut(), rgb.len()) },
            1
        );

        let mut out = [OsviewMatch {
            index: 0,
            score: 0.0,
            p: [0.0; 3],
        }; 1];
        assert_eq!(unsafe { osview_top_k(h, 1, std::ptr::null_mut(), 1) }, -1);
        assert!(last_error(h).contains("out is NULL"));
        assert_eq!(unsafe { osview_top_k(h, 1, out.as_mut_ptr(), 1) }, 1);
        assert_eq!(last_error(h), "");

        unsafe { osview_free(h) };
    }

    #[test]
    fn shape_errors_keep_inputs_for_rescore() {
        let f = fixture("shape");
        let h = osview_new();
        let (ply, feats) = (cpath(&f.ply), cpath(&f.feats));
        let bad_feats =
            std::env::temp_dir().join(format!("osview-ffi-{}-shape-bad.npy", std::process::id()));
        write_npy(&bad_feats, 3, 2, &[1.0, 0.0, 0.0, 1.0, 0.5, 0.5]);
        let bad_path = cpath(&bad_feats);

        let q = [1.0f32, 0.0];
        unsafe {
            assert_eq!(osview_load_ply(h, ply.as_ptr()), 0);
            assert_eq!(osview_load_features(h, bad_path.as_ptr()), 0);
            assert_eq!(osview_set_query(h, q.as_ptr(), 2), 0);
            assert_eq!(osview_score(h), 2);
            assert!(last_error(h).contains("feature rows"));
            assert_eq!(osview_points(h), 4);
            assert_eq!(osview_dim(h), 2);

            // The failed score kept the inputs, so a corrected features file works.
            assert_eq!(osview_load_features(h, feats.as_ptr()), 0);
            assert_eq!(osview_score(h), 0);
            assert_eq!(last_error(h), "");

            // Dim mismatch is caught before the query is accepted.
            let q3 = [1.0f32, 0.0, 0.0];
            assert_eq!(osview_set_query(h, q3.as_ptr(), 3), 0);
            assert_eq!(osview_score(h), 2);
            assert!(last_error(h).contains("query dim"));
            assert_eq!(osview_top_k(h, 1, std::ptr::null_mut(), 0), -2);

            let zero = [0.0f32, 0.0];
            assert_eq!(osview_set_query(h, zero.as_ptr(), 2), 0);
            assert_eq!(osview_score(h), 2);
            assert!(last_error(h).contains("all zeros"));
        }

        let _ = fs::remove_file(&bad_feats);
        unsafe { osview_free(h) };
    }
}
