/*
 * Minimal C usage of the osview SDK: load a PLY, features and a query, score,
 * render 320x240 and print the top-5 matches.
 *
 * Usage:  smoke <points.ply> <features.npy> <query.npy>
 * Build:  cc -I include examples/c/smoke.c -L target/release -losview_rs -o smoke
 *
 * The library parses features.npy itself; this program only needs the query
 * values, so it carries a tiny .npy reader for little-endian float32 arrays
 * (1-D, or 2-D with a single row).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "osview.h"

static const unsigned char NPY_MAGIC[6] = {0x93, 'N', 'U', 'M', 'P', 'Y'};

static int npy_query(const char *path, float **out, size_t *dim_out)
{
    enum { MAX_HEADER = 1 << 20, MAX_DIM = 1 << 28 };
    FILE *f = NULL;
    unsigned char pre[12];
    char *hdr = NULL;
    float *data = NULL;
    size_t dims[2] = {0, 0};
    size_t dim = 0;
    uint32_t hlen;
    int ndim = 0, rc = -1;
    char *p, *shape;
    const char *bad_msg = NULL;

    f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "smoke: cannot open %s\n", path);
        return -1;
    }
    if (fread(pre, 1, 10, f) != 10) {
        bad_msg = "file too short";
        goto bad;
    }
    if (memcmp(pre, NPY_MAGIC, sizeof NPY_MAGIC) != 0) {
        bad_msg = "not a .npy file";
        goto bad;
    }
    if (pre[6] == 1) {
        hlen = (uint32_t)pre[8] | ((uint32_t)pre[9] << 8);
    } else if (pre[6] == 2 || pre[6] == 3) {
        if (fread(pre + 10, 1, 2, f) != 2) {
            bad_msg = "truncated header length";
            goto bad;
        }
        hlen = (uint32_t)pre[8] | ((uint32_t)pre[9] << 8) |
               ((uint32_t)pre[10] << 16) | ((uint32_t)pre[11] << 24);
    } else {
        bad_msg = "unsupported .npy version";
        goto bad;
    }
    if (hlen == 0 || hlen > MAX_HEADER) {
        bad_msg = "bad .npy header length";
        goto bad;
    }
    hdr = malloc((size_t)hlen + 1);
    if (!hdr || fread(hdr, 1, hlen, f) != hlen) {
        bad_msg = "truncated .npy header";
        goto bad;
    }
    hdr[hlen] = '\0';
    if (!strstr(hdr, "'<f4'")) {
        bad_msg = "only little-endian float32 ('<f4') .npy files are supported";
        goto bad;
    }
    if (strstr(hdr, "'fortran_order': True") ||
        strstr(hdr, "\"fortran_order\": True")) {
        bad_msg = "fortran_order is not supported";
        goto bad;
    }
    shape = strstr(hdr, "'shape':");
    if (!shape)
        shape = strstr(hdr, "\"shape\":");
    if (!shape || !(p = strchr(shape, '('))) {
        bad_msg = "missing .npy shape";
        goto bad;
    }
    for (p++; *p && *p != ')';) {
        char *end = NULL;
        unsigned long v;
        while (*p == ' ' || *p == ',')
            p++;
        if (*p == ')' || *p == '\0')
            break;
        if (ndim >= 2) {
            bad_msg = "only 1-D or 2-D .npy arrays are supported";
            goto bad;
        }
        v = strtoul(p, &end, 10);
        if (end == p) {
            bad_msg = "bad .npy shape";
            goto bad;
        }
        dims[ndim++] = (size_t)v;
        p = end;
    }
    if (ndim == 1) {
        dim = dims[0];
    } else if (ndim == 2 && dims[0] == 1) {
        dim = dims[1];
    } else {
        bad_msg = "query must be 1-D or a single row (1, dim)";
        goto bad;
    }
    if (dim == 0 || dim > MAX_DIM) {
        bad_msg = "query dimension out of range";
        goto bad;
    }
    data = malloc(dim * sizeof *data);
    if (!data) {
        bad_msg = "out of memory";
        goto bad;
    }
    if (fread(data, sizeof *data, dim, f) != dim) {
        bad_msg = "truncated .npy payload";
        goto bad;
    }
    *out = data;
    *dim_out = dim;
    data = NULL;
    rc = 0;
bad:
    if (rc != 0)
        fprintf(stderr, "smoke: %s: %s\n", path, bad_msg ? bad_msg : "read error");
    free(hdr);
    free(data);
    fclose(f);
    return rc;
}

int main(int argc, char **argv)
{
    const unsigned W = 320, H = 240;
    Osview *h = NULL;
    OsviewMatch top[5];
    OsviewCamera cam;
    unsigned char *rgb = NULL;
    float *query = NULL;
    size_t qdim = 0, painted = 0, i;
    int rc = 1, n;

#define CHECK(expr)                                                          \
    do {                                                                     \
        int st_ = (expr);                                                    \
        if (st_ != OSVIEW_OK) {                                              \
            fprintf(stderr, "smoke: %s failed (%d): %s\n", #expr, st_,       \
                    osview_last_error(h));                                   \
            goto done;                                                       \
        }                                                                    \
    } while (0)

    if (argc != 4) {
        fprintf(stderr, "usage: %s <points.ply> <features.npy> <query.npy>\n",
                argv[0]);
        return 2;
    }
    if (npy_query(argv[3], &query, &qdim) != 0)
        return 2;

    h = osview_new();
    if (!h) {
        fprintf(stderr, "smoke: osview_new returned NULL\n");
        free(query);
        return 2;
    }
    rgb = malloc((size_t)W * H * 3);
    if (!rgb) {
        fprintf(stderr, "smoke: out of memory\n");
        goto done;
    }

    printf("osview %s\n", osview_version());
    CHECK(osview_load_ply(h, argv[1]));
    CHECK(osview_load_features(h, argv[2]));
    CHECK(osview_set_query(h, query, qdim));
    CHECK(osview_score(h));

    memset(&cam, 0, sizeof cam);
    cam.view = 0;
    cam.width = W;
    cam.height = H;
    cam.radius = 1;
    cam.range_min = 0.05f;
    cam.range_max = 0.10f;
    cam.zoom = 1.0f;
    CHECK(osview_render(h, &cam, rgb, (size_t)W * H * 3));

    for (i = 0; i < (size_t)W * H; i++) {
        const unsigned char *px = rgb + i * 3;
        if (!(px[0] == 24 && px[1] == 24 && px[2] == 28))
            painted++;
    }

    n = osview_top_k(h, 5, top, 5);
    if (n < 0) {
        fprintf(stderr, "smoke: osview_top_k failed (%d): %s\n", n,
                osview_last_error(h));
        goto done;
    }

    printf("points: %zu\n", osview_points(h));
    printf("dim:    %zu\n", osview_dim(h));
    printf("pixels: %zu / %u non-background\n", painted, W * H);
    printf("top_k:  %d\n", n);
    for (i = 0; i < (size_t)n; i++)
        printf("  #%zu index=%u score=%.6f p=(%.4f, %.4f, %.4f)\n", i,
               top[i].index, top[i].score, top[i].p[0], top[i].p[1],
               top[i].p[2]);

    if (osview_points(h) == 0 || painted == 0 || n == 0) {
        fprintf(stderr, "smoke: empty render/ranking\n");
        goto done;
    }
    rc = 0;

done:
    free(rgb);
    free(query);
    osview_free(h);
    return rc;
}

#undef CHECK
