#!/usr/bin/env bash
# One-command verification: CI parity plus optional demo/GUI/C-SDK smoke tests.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
cargo build --release --locked   # emits libosview_rs.{a,dylib,so}
cargo run --release --locked -- --selftest
cargo package --locked --no-verify --allow-dirty

# Optional GUI frontend (needs a windowing backend only at runtime).
cargo build --release --locked --features gui --bin osview-gui

# C SDK: header compiles and the shared library exports the ABI.
tmp="$(mktemp -d -t osview-verify.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
target_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)"
cc -I include -c examples/c/smoke.c -o "$tmp/smoke.o"
cc "$tmp/smoke.o" -L "$target_dir/release" -losview_rs -o "$tmp/smoke"

if rustup toolchain list 2>/dev/null | grep -q '^1\.78'; then
    cargo +1.78 build --release --locked
else
    echo "MSRV toolchain 1.78 missing, skipping msrv stage"
    echo "  install with: rustup toolchain install 1.78 --profile minimal"
fi

if [[ -f demo/sphere.ply && -f demo/feats.npy && -f demo/query.npy ]]; then
    report="$(mktemp -t osview-report.XXXXXX.json)"
    trap 'rm -rf "$tmp" "$report"' EXIT
    cargo run --release --locked -- \
        --points demo/sphere.ply --features demo/feats.npy --query demo/query.npy \
        --out "$tmp/render.png" --report "$report" \
        --width 800 --height 600 --range-min 0.05 --range-max 0.32 >/dev/null
    jq -e '.matches | length > 0' "$report" >/dev/null
    jq -e '.above_min > 0' "$report" >/dev/null
    "$tmp/smoke" demo/sphere.ply demo/feats.npy demo/query.npy
    echo "demo ok"
else
    echo "demo fixtures missing, skipping demo stage (run python3 demo/gen_demo.py)"
fi

echo "verify ok"
