#!/usr/bin/env bash
# Full toolchain rebuild after a sandbox recycle (per scripts/TOOLCHAIN-SETUP.md).
# Logs to /tmp/toolchain-rebuild.log. Idempotent-ish: skips finished stages.
set -uo pipefail
LOG=/tmp/toolchain-rebuild.log
exec > "$LOG" 2>&1
PROJ=/home/z/my-project

echo "== [1/6] rustup (RUSTUP_HOME/CARGO_HOME inside project)"
export RUSTUP_HOME=$PROJ/.rustup
export CARGO_HOME=$PROJ/.cargo
export PATH="$CARGO_HOME/bin:$PATH"
if ! "$CARGO_HOME/bin/rustc" --version 2>/dev/null; then
  curl -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --default-toolchain stable --profile minimal --no-modify-path
fi
rustc --version && cargo --version

echo "== [2/6] windows-msvc target"
rustup target add x86_64-pc-windows-msvc

echo "== [3/6] clang-19 debs -> /tmp/llvmroot"
if ! /tmp/llvmroot/usr/lib/llvm-19/bin/clang-19 --version >/dev/null 2>&1; then
  mkdir -p /tmp/debs /tmp/llvmroot && cd /tmp/debs
  for p in clang-19 libclang-cpp19 libllvm19 lld-19 libclang-common-19-dev llvm-19; do
    curl -sL -m 300 -O "https://deb.debian.org/debian/pool/main/l/llvm-toolchain-19/${p}_19.1.7-3~deb12u1_amd64.deb" &
  done
  wait
  for d in *.deb; do dpkg -x "$d" /tmp/llvmroot; done
  ln -sf clang-19 /tmp/llvmroot/usr/lib/llvm-19/bin/clang-cl
fi
source "$PROJ/scripts/rust-env.sh"
clang-19 --version | head -1

echo "== [4/6] cargo-xwin 0.23.1"
cargo install cargo-xwin --version 0.23.1 --locked
cargo xwin --version

echo "== [5/6] makensis 3.13 debs -> /tmp/nsisx"
if ! /tmp/nsisx/usr/bin/makensis -VERSION >/dev/null 2>&1; then
  mkdir -p /tmp/nsisdeb /tmp/nsisx && cd /tmp/nsisdeb
  curl -sL -m 300 -O "https://deb.debian.org/debian/pool/main/n/nsis/nsis_3.13-1_amd64.deb" \
                  -O "https://deb.debian.org/debian/pool/main/n/nsis/nsis-common_3.13-1_all.deb"
  dpkg -x nsis_3.13-1_amd64.deb /tmp/nsisx
  dpkg -x nsis-common_3.13-1_all.deb /tmp/nsisx
fi
NSISDIR=/tmp/nsisx/usr/share/nsis /tmp/nsisx/usr/bin/makensis -VERSION

echo "== [6/6] tauri CLI (npm global) + xwin SDK prefetch"
npm i -g @tauri-apps/cli >/dev/null 2>&1 && npx --yes @tauri-apps/cli --version || echo "tauri-cli: FAILED (npm)"
# prefetch the Windows SDK/CRT into the persistent xwin cache (~350 MB)
cargo xwin sdk --accept-license 2>&1 | tail -1

echo "== REBUILD DONE — disk:"; df -h / | tail -1
