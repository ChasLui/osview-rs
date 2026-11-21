/*
 * osview.h — C ABI for osview-rs.
 *
 * A handle owns a point cloud (PLY), a per-point feature matrix (NPY), one
 * query vector and the cosine scores derived from them. Feed the handle in
 * small steps, then score once and read the render or the top-k matches.
 *
 * Build the library with `cargo build --release`; it emits
 * target/release/libosview_rs.{a,dylib} (or .so on Linux, .dll/.lib on
 * Windows). Link, for example:
 *
 *     cc -I include app.c -L target/release -losview_rs
 *
 * Status codes (returned by every `int` function except osview_top_k):
 *     OSVIEW_OK         0  success; clears osview_last_error
 *     OSVIEW_ERR_ARG    1  null handle/pointer or invalid argument
 *     OSVIEW_ERR_STATE  2  file parse/load error or wrong handle state
 *
 * Every failure records a UTF-8 message readable through
 * osview_last_error(); every success clears it. The returned pointer is
 * owned by the handle and stays valid until the next call that mutates it.
 *
 * Threading: handle operations are not synchronised. Use one handle per
 * thread, or serialise calls on a handle yourself.
 *
 * Example camera for the demo fixture: 320x240 XY view, radius-1 splats,
 * blue->yellow heat ramp over scores in [0.05, 0.10]:
 *
 *     OsviewCamera cam = {
 *         .view = 0, .width = 320, .height = 240, .radius = 1,
 *         .range_min = 0.05f, .range_max = 0.10f,
 *         .yaw = 0.0f, .pitch = 0.0f, .zoom = 1.0f,
 *         .pan_x = 0, .pan_y = 0,
 *     };
 *     unsigned char rgb[320 * 240 * 3];
 *     if (osview_render(h, &cam, rgb, sizeof rgb) != OSVIEW_OK) { ... }
 */

#ifndef OSVIEW_H
#define OSVIEW_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define OSVIEW_OK 0
#define OSVIEW_ERR_ARG 1
#define OSVIEW_ERR_STATE 2

/* Opaque handle. Create with osview_new, release with osview_free. */
typedef struct Osview Osview;

/* Orthographic camera for osview_render. */
typedef struct OsviewCamera {
    uint32_t view;        /* 0 = xy, 1 = xz, 2 = yz */
    uint32_t width;       /* pixels, 1..=8192 */
    uint32_t height;      /* pixels, 1..=8192 */
    int32_t radius;       /* splat half-size in pixels, 0..=64 */
    float range_min;      /* heat ramp lower bound */
    float range_max;      /* heat ramp upper bound, must be > range_min */
    float yaw;            /* degrees about the vertical screen axis */
    float pitch;          /* degrees about the horizontal screen axis */
    float zoom;           /* > 0, 1 = fit the cloud to the canvas */
    int32_t pan_x;        /* pixels, +right */
    int32_t pan_y;        /* pixels, +up */
} OsviewCamera;

/* One scored point, ordered by osview_top_k. */
typedef struct OsviewMatch {
    uint32_t index;       /* vertex index in the PLY */
    float score;          /* cosine similarity, normally in [-1, 1] */
    float p[3];           /* vertex xyz */
} OsviewMatch;

/* Library version, e.g. "0.2.0". Static storage; do not free. */
const char *osview_version(void);

/* Allocate an empty handle; NULL only on allocation failure. */
Osview *osview_new(void);

/* Release a handle (and its last_error). NULL is a no-op. */
void osview_free(Osview *h);

/* Load a PLY point cloud, replacing any previously loaded cloud.
 * Invalidates the scored dataset (a later osview_score is required).
 * Returns OSVIEW_OK, OSVIEW_ERR_ARG or OSVIEW_ERR_STATE. */
int osview_load_ply(Osview *h, const char *path);

/* Load an NPY feature matrix (little-endian float16/32/64, C order, 1-D or
 * 2-D). Rows are L2-normalized on load and replace any previous matrix.
 * Invalidates the scored dataset. */
int osview_load_features(Osview *h, const char *path);

/* Copy `dim` floats and L2-normalize them, mirroring feature-row
 * normalization; an all-zero query stays zero (and osview_score refuses it).
 * Invalidates the scored dataset. */
int osview_set_query(Osview *h, const float *q, size_t dim);

/* Validate the loaded inputs and compute cosine scores. Requires a PLY, a
 * feature matrix and a query; a feature row count != point count, a query
 * dimension mismatch or an all-zero query fails with OSVIEW_ERR_STATE.
 * Calling it again while the dataset is already scored is a no-op success. */
int osview_score(Osview *h);

/* Render the point cloud into tightly packed RGB8 (`rgb_len` bytes,
 * at least width*height*3; extra bytes are left untouched).
 *
 * Requires a loaded point cloud. When osview_score has run the heat ramp
 * reflects the scores; otherwise points keep their PLY color (neutral gray
 * when the PLY has none). Background is RGB (24, 24, 28).
 *
 * Validates the camera (view 0..2, width/height 1..=8192, radius 0..=64,
 * finite yaw/pitch, finite zoom > 0, range_max > range_min) and the buffer. */
int osview_render(const Osview *h, const OsviewCamera *cam, uint8_t *rgb,
                  size_t rgb_len);

/* Write the best `min(k, out_len, points)` matches, score-descending (ties
 * keep ascending vertex index), and return that count. Requires osview_score
 * to have run. On error returns a negative value and records last_error:
 *     -1  null/invalid argument (e.g. out == NULL with out_len > 0)
 *     -2  not scored yet
 */
int osview_top_k(const Osview *h, size_t k, OsviewMatch *out, size_t out_len);

/* Point count of the loaded PLY (0 when none or h is NULL). */
size_t osview_points(const Osview *h);

/* Feature dimension of the loaded matrix (0 when none or h is NULL). */
size_t osview_dim(const Osview *h);

/* Last error message: "" when clear, "null handle" when h is NULL.
 * Owned by the handle; do not free. */
const char *osview_last_error(const Osview *h);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OSVIEW_H */
