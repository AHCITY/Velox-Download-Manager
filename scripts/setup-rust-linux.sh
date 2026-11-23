#!/usr/bin/env bash
# Linux-only Rust toolchain for running the CI gates locally (fmt / clippy / test).
# Lightweight alternative to rebuild-toolchain.sh, which also builds the whole
# Windows cross-toolchain (clang-19, cargo-xwin, xwin SDK, makensis).
#
# Installs stable + rustfmt + clippy into the git-ignored .rustup/.cargo inside
# the project (RUSTUP_HOME/CARGO_HOME per scripts/rust-env.sh), so nothing is
# written outside the workspace. Idempotent: a working toolchain is kept.
set -euo pipefail
PROJ=/home/z/my-project
export RUSTUP_HOME="$PROJ/.rustup"
export CARGO_HOME="$PROJ/.cargo"
mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"

if ! "$CARGO_HOME/bin/cargo" --version >/dev/null 2>&1; then
  echo "== installing rustup (stable, minimal + rustfmt + clippy) into $RUSTUP_HOME"
  curl -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --default-toolchain stable --profile minimal \
     --component rustfmt --component clippy --no-modify-path
fi

export PATH="$CARGO_HOME/bin:$PATH"
echo "rustc  $(rustc --version)"
echo "cargo  $(cargo --version)"
echo "fmt    $(cargo fmt --version)"
echo "clippy $(cargo clippy --version)"
echo "disk:  $(df -h / | awk 'NR==2 {print $4 " free on /"}')"
