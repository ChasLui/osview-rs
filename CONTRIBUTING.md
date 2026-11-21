# Contributing

Thanks for taking a look at `osview-rs`. It is a single-binary utility; the
bar for new dependencies, runtime layers or configuration is intentionally high.

## Build and test

```bash
cargo build --release
cargo test --release
cargo run --release -- --selftest
```

Before opening a PR, run the Rust checks CI runs:

```bash
scripts/verify.sh
```

## Pull requests

- Keep changes focused; one concern per PR.
- Behavior changes need a regression test when the affected path is not already
  covered (`src/npy.rs`, `src/ply.rs` carry `#[cfg(test)]` suites; `--selftest`
  covers scoring and rendering end to end).
- Update `README.md` when CLI flags, input formats or output JSON change, and
  add a `CHANGELOG.md` entry under `[Unreleased]`.
- Do not commit generated data (`demo/*.ply`, `*.npy`, `*.png`, `*.json` are
  git-ignored); regenerate with `demo/gen_demo.py` when needed.

## Commit messages

Imperative mood, one line, optionally a body explaining the why:
`fix: reject non-finite ascii PLY values`, `docs: document report keys`.

## Reporting bugs

Open an issue with the exact command, the tool version/commit, and a
description of the inputs. If the inputs are private, state the format
(dtypes, shape, PLY property list) instead of attaching them.
