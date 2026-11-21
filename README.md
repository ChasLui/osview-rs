# osview-rs

![demo render](docs/render.png)

Headless rewrite of OpenScene's `osview` for machine use: reads a PLY point
cloud plus per-point CLIP features, scores one query vector, and writes a PNG
and a JSON top-k report. No display, no GLUT, no GPU.

Reproduced semantics (from `osview.cpp` in the OpenScene demo):

- the i-th feature row belongs to the i-th PLY vertex (pairing is by index, not name);
- feature rows are L2-normalized on load; zero rows stay zero and never match;
- score = dot product of the row with the (normalized) query vector = cosine;
- points with `score < --range-min` keep their PLY vertex color; the rest use a
  blue→yellow ramp between `--range-min` and `--range-max` (osview defaults
  0.05 / 0.10).

Not implemented: the interactive window, camera orbit/zoom, CLIP text encoding
(pass a `.npy` text embedding instead), `.ssa/.conf` scenes, per-image feature
files (`-image_directory`).

## Build

```bash
cargo build --release
# binary: target/release/osview-rs (honours CARGO_TARGET_DIR)
```

## Usage

```bash
osview-rs --points region0.ply --features RPmz2sHmrrY_region0.npy \
          --query chair.npy --out chair.png --report chair.json \
          --view xy --width 1600 --height 1200 --range-min 0.05 --range-max 0.10
```

Inputs

- `--points`: PLY, `ascii` or `binary_little_endian`, vertex `x y z` plus
  optional `red green blue` (uchar or float 0-1).
- `--features`: `.npy` `[N, D]` float16/32/64, C order. `D` is whatever width
  produced them (CLIP ViT-L/14@336px → 768).
- `--query`: `.npy` `[1, D]`, `[D, 1]` or `[D]`. `clip_server.py` writes
  `[1, 768]` float32.

Outputs

- PNG (`--out`): orthographic projection on `xy`, `xz` or `yz`; depth buffer
  keeps the nearest splat; `--radius` controls point size.
- JSON report (`--report`): `points`, `dim`, `view`, `range`, `above_min`,
  and `matches` (index, score, xyz) for the top `--topk` points.
- stdout: one-line JSON summary with the top 10.

Query vector from OpenScene's helper:

```bash
python3 demo/clip_server.py --out_dir tmp &   # writes tmp/<query>.npy
# or any CLIP text encoder producing the same 768-d space
```

## Tests

```bash
cargo test --release     # NPY parse/normalize, binary PLY
cargo run --release -- --selftest   # ranking, render coverage, ascii PLY
```

## Demo

`demo/gen_demo.py` builds a 40k-point sphere with 768-d float16 features
(the query highlights the +x hemisphere). Generated data is git-ignored:

```bash
cd demo && python3 gen_demo.py && cd ..
osview-rs --points demo/sphere.ply --features demo/feats.npy \
          --query demo/query.npy --out demo/render.png \
          --width 800 --height 600 --range-min 0.05 --range-max 0.32 \
          --report demo/top.json
```

## Notes

- Point splatting, not surface rasterization: thin geometry shows as dots.
  Increase `--radius` or resolution for denser coverage.
- Views are axis-aligned; for arbitrary camera poses render with the engine
  and reuse only the scoring/report part.
- Scoring is single-query. For class argmax, loop over class vectors and keep
  the max per point, then reuse the same renderer.