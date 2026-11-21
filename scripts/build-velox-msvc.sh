#!/bin/bash
# Velox Windows cross-build: cargo-xwin + Debian clang-19 (clang-cl) + lld-link.
# Produces app/src-tauri/target/x86_64-pc-windows-msvc/release/Velox.exe
# Recipe details: scripts/TOOLCHAIN-SETUP.md (clang resource dir is critical).
set -e
source /home/z/my-project/scripts/rust-env.sh

export PATH=/tmp/llvmroot/usr/lib/llvm-19/bin:$PATH
export LD_LIBRARY_PATH=/tmp/llvmroot/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}
export XWIN_CACHE_DIR=/home/z/my-project/.xwin-cache
mkdir -p "$XWIN_CACHE_DIR"

cd /home/z/my-project/download-engine/app/src-tauri
exec cargo xwin build --release --target x86_64-pc-windows-msvc "$@"
