#!/usr/bin/env bash
# Every gate CI and pre-commit run, in order. Fails on the first problem.
set -euo pipefail
cd "$(dirname "$0")/.."

echo '== rustfmt'
cargo fmt --all --check

echo '== clippy'
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings

echo '== driver lint policy matches the root workspace'
lints() { awk '/^\[workspace\.lints/{on=1} /^\[/&&!/^\[workspace\.lints/{on=0} on && NF' "$1"; }
diff <(lints Cargo.toml) <(lints crates/driver/Cargo.toml)

echo '== driver rustfmt (own workspace and nightly toolchain)'
(cd crates/driver && cargo fmt --check)

echo '== driver clippy'
(cd crates/driver && cargo clippy --all-targets --quiet -- -D warnings)

echo '== ast-grep rule tests'
ast-grep test

echo '== ast-grep scan'
ast-grep scan crates

echo 'all checks passed'
