# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `osview_rs` library: `Dataset` load/validate/score/top-k, json report and
  stdout summary writers shared by every frontend.
- `osview-gui` (feature `gui`, minifb): live orbit/pan/zoom, view and ramp
  keys, `--frames` smoke mode.
- C SDK: `include/osview.h`, `libosview_rs` (rlib/cdylib/staticlib) with an
  opaque handle, camera/matches structs and a complete `examples/c/smoke.c`.
- CI: C-SDK link check and a macOS GUI job.

### Changed

- The CLI is now a thin frontend over the library; its PNG, report, stdout and
  exit codes are unchanged.

### Fixed

- `demo/gen_demo.py` stacked coordinates on the wrong axis, collapsing the
  sphere into a line; the fixed script and `docs/render.png` are regenerated.

## [0.2.0] - 2026-11-21

### Added

- `--yaw`, `--pitch`, `--zoom`, `--pan-x`, `--pan-y` for deterministic camera
  framing; the report records `camera`.

### Fixed

- Depth buffer keeps f64 rotated depths, so occlusion stays correct beyond
  `f32::MAX`.

## [0.1.0] - 2026-11-21

### Added

- Headless rewrite of OpenScene's `osview`: reads a PLY point cloud plus
  per-point NPY features, scores one query vector by cosine, and writes a PNG
  render and a JSON top-k report; no display, GLUT or GPU.
- Orthographic `xy`/`xz`/`yz` views with a depth buffer and a blue→yellow heat
  ramp over `--range-min`..`--range-max`.
- NPY reader (`<f2`/`<f4`/`<f8`, C order, 1-D or 2-D) with L2 row
  normalization; PLY reader for `ascii` and `binary_little_endian` with
  optional `red green blue` (uchar/ushort/float).
- `--selftest` end-to-end check plus unit tests for the NPY and PLY parsers.
- GitHub Actions CI (fmt, clippy `-D warnings` on all targets, tests,
  selftest, docs lint, zizmor on PRs) and a tag-driven release workflow for
  Linux x86_64 (static musl) and macOS arm64 with sha256 checksums.

[Unreleased]: https://github.com/ChasLui/osview-rs/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/ChasLui/osview-rs/releases/tag/v0.2.0
[0.1.0]: https://github.com/ChasLui/osview-rs/releases/tag/v0.1.0
