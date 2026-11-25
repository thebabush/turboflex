#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

cargo build --locked --release -p turboflex --target-dir "$out/cli"
(cd crates/driver && cargo build --locked --release --target-dir "$out/driver")
for workspace in . crates/driver; do
    "$out/driver/release/turboflex-driver" "$workspace" --out "$out/facts" --locked
    "$out/cli/release/turboflex" check --facts "$out/facts" turboflex.kdl turboflex-construction.kdl
done
