# Windows cross-build toolchain — full rebuild recipe

Everything under `/tmp` dies with the sandbox; this is the proven recipe to
rebuild it (used 2026-11-21 twice). Host: Debian userland, glibc 2.41 — Debian
debs run natively. Disk need: ~600 MB in /tmp.

## 1. clang-19 + lld-link (cargo-xwin's compiler/linker)

```bash
mkdir -p /tmp/debs /tmp/llvmroot && cd /tmp/debs
# parallel download from Debian pool (bookwork build set, any consistent set works):
for p in clang-19 libclang-cpp19 libllvm19 lld-19 libclang-common-19-dev llvm-19; do
  curl -sL -m 300 -O "https://deb.debian.org/debian/pool/main/l/llvm-toolchain-19/${p}_19.1.7-3~deb12u1_amd64.deb" &
done; wait
for d in *.deb; do dpkg -x "$d" /tmp/llvmroot; done
# CRITICAL: libclang-common-19-dev ships the builtin headers (stddef.h ...) in
# /usr/lib/llvm-19/lib/clang/19/include — without it ring/any C build script dies
# with "'stddef.h' file not found". This is the "resource dir is critical" trap.
# CRITICAL 2: llvm-19 ships llvm-lib — without it cc-rs dies with
# "failed to find tool llvm-lib" (ring build script).

# cargo-xwin invokes clang-cl; Debian doesn't ship it as a symlink — argv0 dispatch:
ln -sf clang-19 /tmp/llvmroot/usr/lib/llvm-19/bin/clang-cl
# smoke test (rust-env.sh already puts /tmp/llvmroot on PATH/LD_LIBRARY_PATH):
source /home/z/my-project/scripts/rust-env.sh && clang-19 --version
```

## 2. cargo-xwin (0.23.1, installed into the persistent CARGO_HOME)

```bash
source /home/z/my-project/scripts/rust-env.sh
cargo install cargo-xwin --version 0.23.1 --locked   # ~4 min compile
cargo xwin --version
```

The Windows SDK + CRT live in the persistent `/home/z/my-project/.xwin-cache`
(survives recycles; only re-download if it was deleted: `XWIN_CACHE_DIR` is set
in rust-env.sh, cargo-xwin fetches on first build).

## 3. makensis 3.13 (installer packaging)

```bash
mkdir -p /tmp/nsisdeb /tmp/nsisx && cd /tmp/nsisdeb
curl -sL -m 300 -O "https://deb.debian.org/debian/pool/main/n/nsis/nsis_3.13-1_amd64.deb" \
                -O "https://deb.debian.org/debian/pool/main/n/nsis/nsis-common_3.13-1_all.deb"
dpkg -x nsis_3.13-1_amd64.deb /tmp/nsisx
dpkg -x nsis-common_3.13-1_all.deb /tmp/nsisx
NSISDIR=/tmp/nsisx/usr/share/nsis /tmp/nsisx/usr/bin/makensis -VERSION   # v3.13-1
```

## 4. Build + package

```bash
/home/z/my-project/scripts/build-velox-msvc.sh          # → app/src-tauri/target/x86_64-pc-windows-msvc/release/Velox.exe
NSISDIR=/tmp/nsisx/usr/share/nsis /tmp/nsisx/usr/bin/makensis \
  /home/z/my-project/scripts/installer-unified-template.nsi  # → /tmp/installer/Velox-Setup-x64.exe (the ONE unified installer, Phase 5)
```

OutFile path is fixed in the template (`/tmp/installer/`); create it first.
See PROGRESS-LOG 2026-11-21 for the release-upload flow (delete old assets,
upload, update notes, round-trip hash verify).
