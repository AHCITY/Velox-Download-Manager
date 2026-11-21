#!/bin/bash
# Rust environment for all Velox builds. Source this file.
export RUSTUP_HOME=/home/z/my-project/.rustup
export CARGO_HOME=/home/z/my-project/.cargo
export PATH="$CARGO_HOME/bin:/tmp/llvmroot/usr/lib/llvm-19/bin:$PATH"
export LD_LIBRARY_PATH=/tmp/llvmroot/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}
export XWIN_CACHE_DIR=/home/z/my-project/.xwin-cache
