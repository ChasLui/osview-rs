#!/usr/bin/env bash
# One-command verification: CI parity plus an optional demo render.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
cargo run --release --locked -- --selftest
cargo package --locked --no-verify --allow-dirty

if rustup toolchain list 2>/dev/null | grep -q '^1\.78'; then
  cargo +1.78 build --release --locked
else
  echo "MSRV toolchain 1.78 missing, skipping msrv stage"
  echo "  install with: rustup toolchain install 1.78 --profile minimal"
fi

if [[ -f demo/sphere.ply && -f demo/feats.npy && -f demo/query.npy ]]; then
    report="$(mktemp -t osview-report.XXXXXX.json)"
    trap 'rm -f "$report"' EXIT
    cargo run --release --locked -- \
        --points demo/sphere.ply --features demo/feats.npy --query demo/query.npy \
        --out "$(mktemp -t osview-render.XXXXXX.png)" --report "$report" \
        --width 800 --height 600 --range-min 0.05 --range-max 0.32 >/dev/null
    jq -e '.matches | length > 0' "$report" >/dev/null
    jq -e '.above_min > 0' "$report" >/dev/null
    echo "demo ok"
else
    echo "demo fixtures missing, skipping demo stage (run python3 demo/gen_demo.py)"
fi

echo "verify ok"
