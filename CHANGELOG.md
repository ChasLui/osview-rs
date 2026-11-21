# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/ChasLui/osview-rs/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ChasLui/osview-rs/releases/tag/v0.1.0
