# AGENTS.md

`osview-rs` replaces OpenScene's `osview` core: it reads a PLY point cloud plus
per-point CLIP features (NPY), scores one query vector, and renders/top-k's the
result. One crate, one renderer, four surfaces: CLI (`osview-rs`), GUI
(`osview-gui`, feature `gui`), C SDK (`libosview_rs` + `include/osview.h`) and
the Rust library (`osview_rs`). Edition 2021; dependency budget: `png`, plus
optional `minifb` for the GUI. No async/runtime/GPU. Style contract: no clap,
serde, anyhow, rayon, ndarray — errors are `io::Result` / `Result<_, String>`,
JSON is hand-written `write!`, tests hand-build NPY bytes. Adding a dependency
is a design decision, not a refactor.

## Commands (CI parity: `.github/workflows/ci.yml`)

```bash
cargo build --release            # binary honours CARGO_TARGET_DIR
cargo fmt --check
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release             # 23 unit tests: npy, ply, render, Dataset, ffi
cargo run --release -- --selftest   # ranking + render coverage + ascii PLY
cargo build --release --features gui --bin osview-gui
cargo clippy --release --locked --features gui --all-targets -- -D warnings
cc -I include examples/c/smoke.c -L target/release -losview_rs -o /tmp/osview-smoke
```

CI (`.github/workflows/ci.yml`) runs these plus an MSRV build
(`cargo +1.78 build --release --locked`), a C-SDK link check, a GUI job on
macOS (`clippy --features gui --all-targets`), a docs job
(`yamllint`, `markdownlint-cli2`, `actionlint`) and, on PRs,
`zizmor` over the workflows.
Every action is pinned by full commit SHA with a `# vX.Y.Z` comment; Dependabot
(grouped, 7-day cooldown) bumps them. `release.yml` creates the GitHub Release
itself and builds `x86_64-unknown-linux-musl` + `aarch64-apple-darwin` with
`--locked`.

Demo fixtures are git-ignored; regenerate before running the demo:

```bash
cd demo && python3 gen_demo.py && cd ..
cargo run --release -- --points demo/sphere.ply --features demo/feats.npy \
  --query demo/query.npy --out demo/render.png --width 800 --height 600 \
  --range-min 0.05 --range-max 0.32 --report demo/top.json
```

Expected: top score ≈ 0.9699, the noise floor ≈ 0.0652, `above_min` > 0, the
+x hemisphere yellow. `docs/render.png` is the committed screenshot; the demo
run does not update it.

## Layout and data flow

- `src/lib.rs` — `Dataset` (load/validate/score/top-k), `write_report`,
  `summary_json`; the shared contract every surface calls.
- `src/main.rs` — CLI only: `Opts`, `parse_args` (manual `--flag value`),
  `run()` wiring, `mod selftest`. Arg errors exit 2 (`die`/`usage`); runtime
  errors exit 1; `-h/--help` exits 0.
- `src/gui/main.rs` — `osview-gui` (feature `gui`, minifb): orbit/pan/zoom,
  view and ramp keys over the same renderer; `--frames N` renders N frames,
  prints the painted count and exits for smoke tests. `scripts/bundle-gui.sh`
  wraps the binary in `dist/osview-rs.app` (Info.plist, version from cargo
  metadata); a bare executable cannot reliably become the frontmost app, so
  `open -a dist/osview-rs.app` is the supported way to launch it for mouse and
  keyboard testing. Input mapping (`key_action`/`drag_step`/`wheel_step`) is
  unit-tested under `--features gui`; OS event delivery itself is not testable
  from scripts (synthetic keystrokes only land when the window is the active
  key window — verify that manually).
- `src/ffi.rs` — C ABI; the only module allowed `unsafe` (`allow` inside the
  module, crate lint is `deny`). `include/osview.h` and
  `examples/c/smoke.c` document/exercise it.
- `src/npy.rs` — minimal NPY parser (`<f2`/`<f4`/`<f8`, C order, f64 downcast
  to f32), L2 row normalization. Header is parsed by string scanning, not a
  real NPY parser.
- `src/ply.rs` — PLY parser (`ascii`, `binary_little_endian`, vertex element
  first), color conversion.
- `src/render.rs` — orthographic splat renderer with depth buffer and heat ramp.

```text
CLI argv -> parse_args -> run \
GUI argv -> Dataset::load        > Dataset{cloud, features, query, scores}
C SDK    -> handle stages       /
  Dataset::load: ply::read -> Cloud; npy::read -> Matrix (rows normalized)
  Dataset::from_parts: row/dim checks, all-zero query refusal, dot scores
  render::render -> RGB8 buffer -> write_png (CLI) / u32 frame (GUI) /
  caller buffer (C SDK); CLI adds --report JSON + one stdout JSON line
```

## Invariants (do not break)

- Index pairing: feature row `i` belongs to PLY vertex `i`; `rows == points` is
  enforced before scoring. Any filter/compaction must apply to both sides.
- Normalization: feature rows and query are both L2-normalized at load, so the
  dot product is cosine. Rows whose norm is non-finite or `<= 1e-12` become all
  zeros silently — NaN/±Inf feature rows score 0, they do not error. PLY
  coordinates, by contrast, error on non-finite values.
- Zero-query refusal lives in `Dataset::from_parts`, so CLI, GUI and C SDK all
  refuse it (a NaN/Inf query is zeroed by the loader and hits the same error).
- Color rule: heat ramp iff `score >= range_min`; `t` saturates at `range_max`;
  `above_min` counts `>=`. Below the threshold the PLY vertex color is kept —
  only when all of `red/green/blue` exist, otherwise gray `[190,190,190]`
  (background is `[24,24,28]`).
- Depth buffer keeps the *largest* rotated coordinate on the depth axis (camera
  on the +axis), stored as f64 so depths beyond `f32::MAX` still order; ties
  resolve to the later point (`>=`). Do not "correct" to smaller-is-nearer.
- Splat is a square `(2r+1)^2` clipped to the canvas; `--radius 0` is one pixel.
- Autoscale is global and uniform over the two projected axes with a 4 % margin,
  centered, y flipped; one outlier reframes the whole image.
- Ranking is a stable descending `total_cmp` sort: equal scores keep ascending
  index. `--topk` affects only `--report`; stdout always emits top 10.
- Output keys are `i`, `score`, `p` (not index/xyz). JSON must stay valid: no
  NaN/inf may reach output, paths are JSON-escaped. Report scores 6dp; stdout
  `max`/`min` 6dp, `top` entries 4dp.
- Validation: width/height `1..=8192`, radius `0..=64`, `range_max > range_min`
  and both finite; `--points/--features/--query/--out` mandatory, `--report`
  optional. Defaults: xy, 1600×1200, radius 1, 0.05/0.10, topk 500.
- Parser forms: `--flag value` only (`--width=800` is an unknown argument),
  repeated flags last-wins, missing value exits 2. stdout EPIPE is not an
  error (`osview-rs ... | head` exits 0).

- Camera: rotations are applied to (horizontal, vertical, depth) coordinates
  before autoscale and in f64 (f32 overflows near `f32::MAX`); `--zoom`
  multiplies the fitted scale; `--pan-*` shifts final pixels. Defaults
  (`0/0/1/0/0`) must reproduce the axis-aligned projection.
- Surface parity: CLI, GUI and C SDK share `Dataset` + `render`; do not fork
  scoring, validation or the color rule. CLI output must stay byte-identical
  when the library is refactored (demo PNG + report compared).
- C ABI: status 0 ok, 1 null/invalid argument, 2 load/state error; every
  failure sets `osview_last_error` (empty on success). Paths are NUL-terminated
  UTF-8; output buffers are caller-owned and length-checked. FFI never panics
  across the boundary and never invents defaults for a missing stage.

## Scope limits (intentional)

- PLY: `ascii` and `binary_little_endian` only; vertex element must come first;
  no list properties, no big-endian.
- NPY: little-endian floats only; no `fortran_order`, no structured dtypes;
  extra trailing payload bytes are accepted, short payloads are rejected.
- CLI views are axis-aligned orthographic with deterministic framing via
  `--yaw/--pitch/--zoom/--pan-x/--pan-y`; the GUI adds live orbit on the same
  camera. No GPU rasterization, no CLIP text encoding, no `.ssa/.conf` scenes,
  no per-image feature directories. Feature width is whatever the producer
  emitted — never hard-code 768; `feats.dim` is authoritative.

## Reproduction semantics

Upstream reference is `osview.cpp` in the OpenScene demo (not vendored here —
do not cite upstream line numbers). Keep scoring, threshold (0.05/0.10) and
blue→yellow ramp direction identical to upstream; the ramp is a direction
reproduction, not pixel-exact. Fix only genuine defects.

## Sharp edges (document, don't silently fix)

Framing/rotation math is f64: default framing can differ from v0.1.0 by a
pixel or more when the bbox midpoint is not f32-representable; rotating
clouds whose coordinates are much larger than 1e7 amplifies f32 ulp
residual (recentre them first). Full-file reads capped at 8 GiB
(non-regular inputs rejected); depth buffer
is f64 (8 B/px, 537 MB at 8192²); single-threaded
O(N·D) scoring; square splats, `--radius` capped at 64; no alpha; PNG is
written before the report (a report failure leaves a valid PNG and exit 1);
`--out` is not atomic and must not alias an input or `--report`; `--out`
extension unchecked; tests/selftest use fixed temp filenames and can race
across concurrent runs; `usage()` version comes from `CARGO_PKG_VERSION`.
GUI needs a windowing backend at runtime (build/`--frames` smoke still opens a
window); the C SDK cannot parse files itself beyond the paths it is handed, and
its camera validation mirrors the CLI (w/h 1..=8192, radius 0..=64, finite
yaw/pitch/zoom>0, range_max>range_min).

## Verification

Run `scripts/verify.sh` before claiming a change is done: fmt + clippy
(`--all-targets`) + tests + `--selftest`, a GUI build, a C SDK compile/link,
and — when the git-ignored demo fixtures exist — a demo render gated with `jq`
plus the C smoke test. For CLI/report changes also validate the report against
`docs/report.schema.json`. For parser changes add a hand-built byte fixture
following the `npy_f32` / `npy_f16` helpers; do not add dev-dependencies. Never
commit generated demo data. GUI changes: at least `--frames 2` locally, and
confirm the window renders non-background pixels.
GUI changes additionally: `cargo test --features gui` (input mapping) and
`scripts/bundle-gui.sh` so the bundle stays buildable.

The runtime-facing usage guide for agents is `skills/osview-rs/SKILL.md`
(symlinked at `.claude/skills/osview-rs`); keep its flag table and error table
in sync with `usage()` and the messages in `src/`.
