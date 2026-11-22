---
name: osview-rs
license: MIT
description: Scores a text-embedding query against per-point CLIP features and renders the result with osview-rs (PLY point cloud + .npy feature matrix). Use when working with OpenScene region*.ply + per-region .npy feature pairs, producing heat-map PNG renders, or generating top-k match JSON on machines without a display or GPU.
---

# osview-rs

Headless replacement for OpenScene's `osview`. Reads a PLY point cloud and an
`[N, D]` NPY feature matrix, scores one query vector by cosine, and writes a
PNG render plus a JSON top-k report.

## When to use

- Locate points matching a text query in a feature-rich point cloud (OpenScene
  `region*.ply` + `RPmz....npy` pairs).
- Produce a heat-mapped render or a machine-readable top-k list.
- Reproduce osview scoring without a display, GLUT or GPU.

Not for: CLIP text encoding (produce the `.npy` query with a CLIP encoder
first), `.ssa/.conf` scenes.

## Quick start

All paths below are repo-root relative: run from the repository root
(`cd "$(git rev-parse --show-toplevel)"`). If this skill is installed
outside the repo, substitute the path to a local checkout.

```bash
python3 demo/gen_demo.py      # 40k-point sphere, 768-d float16 features
cargo run --release -- \
  --points demo/sphere.ply --features demo/feats.npy --query demo/query.npy \
  --out demo/render.png --report demo/top.json \
  --width 800 --height 600 --range-min 0.05 --range-max 0.32
```

Build once with `cargo build --release` (binary honours `CARGO_TARGET_DIR`) or
`cargo install --path .`.

## Flags

| flag | default | notes |
| --- | --- | --- |
| `--points` | required | PLY ascii/binary_little_endian, vertex first |
| `--features` | required | .npy [N, D], f16/f32/f64, C order |
| `--query` | required | `.npy` `[1, D]` or `[D]`; same width as the features |
| `--out` | required | PNG path |
| `--report` | none | JSON top-k path |
| `--view` | `xy` | `xy`, `xz` or `yz` |
| `--width` / `--height` | 1600 / 1200 | 1..=8192 |
| `--radius` | 1 | square half-width in pixels, 0..=64 |
| `--yaw` / `--pitch` | 0 / 0 | rotate about vertical / horizontal screen axis, degrees |
| `--zoom` | 1 | scale about the image centre, > 0 |
| `--pan-x` / `--pan-y` | 0 / 0 | pixel shift, right/up positive |
| `--range-min/max` | 0.05/0.10 | heat threshold and ramp top |
| `--topk` | 500 | entries written to `--report` |
| `--selftest` | — | built-in end-to-end check, then exit |

- `--points` needs `x y z`; `red green blue` is optional.
- `--features` rows must equal the vertex count; C order.
- `--range-min/--range-max` need `max > min`, both finite.

Exit codes: 2 for usage/validation errors, 1 for runtime failures, 0 success.

## Other surfaces

The same renderer is available without re-implementing scoring:

- GUI: `cargo build --release --features gui`, then `osview-gui --points ...
  --features ... --query ...`. Drag orbits, right/shift-drag pans, wheel zooms,
  `1`/`2`/`3` switch views, `[`/`]` move the ramp top, `r` resets, `q` quits.
  `--frames N` renders N frames, prints the painted-pixel count and exits.
  On macOS run `scripts/bundle-gui.sh` and `open -a dist/osview-rs.app --args
  ...` when the window must hold keyboard focus; a bare binary does not.
- C SDK: build once, then link `-losview_rs` with `include/osview.h`; open a
  handle, load the three inputs, `osview_score`, then `osview_render` into your
  own RGB buffer or `osview_top_k`. See `examples/c/smoke.c`.
- Rust: `osview_rs::{Dataset, Camera, View, render}`.

## Outputs

Report JSON (schema: `docs/report.schema.json`):

```json
{"points":40000,"dim":768,"view":"xy","camera":{"yaw":0,"pitch":0,"zoom":1,"pan":[0,0]},
 "range":[0.05,0.32],"above_min":123,
 "matches":[{"i":17,"score":0.963700,"p":[0.900000,0.100000,0.200000]}]}
```

stdout: one JSON line with `points`, `dim`, `view`, `image`, `above_min`,
`max`, `min`, and `top` (10 entries, 4 decimals).

jq recipes:

```bash
jq '.matches[0]' top.json          # best match
jq '[.matches[].i]' top.json       # indices only
jq -e '.above_min > 0' top.json    # gate for scripts
```

## Scoring semantics

- Feature row `i` belongs to PLY vertex `i`; pairing is positional.
- Rows and the query are L2-normalized on load, so score = cosine.
- Zero and non-finite rows become all zeros silently (they never match).
- `score >= --range-min` gets the blue→yellow ramp; below it keeps the PLY
  vertex color (grey when the cloud has no full RGB triplet).
- Camera: yaw/pitch rotate the point set before autoscale, zoom multiplies
  the fitted scale, pan shifts pixels; the report records the camera.
- An all-zero query is rejected. A `[D, 1]` column vector is not a valid query;
  use `[1, D]` or `[D]`.

## Common errors

| message | fix |
| --- | --- |
| `feature rows (N) != points (M)` | pass matching files; no reordering |
| `query dim (a) != feature dim (b)` | same CLIP tower for both |
| `query must be a single vector` | `[1, D]` or `[D]`, not `[D, 1]` |
| `query vector is all zeros` | pass a real embedding |
| `PLY: vertex element must come first` | re-export vertices first |
| `not a regular file` | pass a file, not a FIFO or device |
| `input larger than 8 GiB` | slice or down-sample the input |
| `--out must not overwrite an input file` | pick another output path |
| `--out and --report must differ` | use distinct paths |
| `yaw/pitch must be finite` | pass finite degrees |
| `zoom must be finite and > 0` | pass a positive zoom factor |

## Verify

```bash
scripts/verify.sh
```

Runs fmt, clippy (all targets), tests, `--selftest`, a GUI build, a C-SDK
compile/link, and (when demo fixtures exist) a demo render with the report
gated by `jq` plus the C smoke test.
