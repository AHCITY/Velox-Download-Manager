#!/usr/bin/env python3
"""Audit: measure compressibility of the REAL staged installer payload.

Answers objective 4 (installer size optimization) with measured numbers:
- per-file zstd/lzma compression ratios for every staged component
- ffmpeg.exe vs ffprobe.exe cross-compression (how much the SECOND copy
  really costs under solid/solid-LZMA compression)
- a full solid-LZMA recompress of the installer payload -> what
  "SetCompressor /SOLID lzma" would have produced for the same files.
"""
import lzma
import os
import sys
import time

STAGING = '/tmp/installer'
FILES = [
    'Velox.exe', 'yt-dlp.exe', 'ffmpeg.exe', 'ffprobe.exe',
    'MicrosoftEdgeWebview2Setup.exe', 'velox-browser-host.exe',
]

def ratio(name, data, comp, level, label):
    t0 = time.time()
    out = comp(data)
    dt = time.time() - t0
    print(f"  {label:<28} {len(out):>12,} bytes  ratio {len(data)/max(1,len(out)):5.2f}x  ({dt:5.1f}s)")
    return len(out)

print("=== per-file compressibility (real staged bytes) ===")
sizes = {}
for f in FILES:
    p = os.path.join(STAGING, f)
    if not os.path.exists(p):
        print(f"  {f}: MISSING at {p}")
        continue
    data = open(p, 'rb').read()
    sizes[f] = len(data)
    print(f"{f}: {len(data):,} bytes")
    # zstd-like quick check with lzma at low level is too slow for 138MB x2;
    # use zlib (what NSIS default uses) and lzma with 1 thread
    import zlib
    ratio(f, data, lambda d: zlib.compress(d, 6), 6, 'zlib-6 (NSIS default)')
    if len(data) < 40 * 1024 * 1024:
        ratio(f, data, lambda d: lzma.compress(d, preset=6), 6, 'lzma-6 alone')
    else:
        # big files: preset 6, no extreme
        ratio(f, data, lambda d: lzma.compress(d, preset=6), 6, 'lzma-6 alone')

print("\n=== ffmpeg.exe vs ffprobe.exe: cost of the second copy ===")
ff = open(os.path.join(STAGING, 'ffmpeg.exe'), 'rb').read()
fp = open(os.path.join(STAGING, 'ffprobe.exe'), 'rb').read()
both = ff + fp
t0 = time.time()
both_lzma = lzma.compress(both, preset=6)
dt = time.time() - t0
ff_alone = lzma.compress(ff, preset=6)
print(f"  ffmpeg+ffprobe concatenated ({len(both):,}): lzma -> {len(both_lzma):,} ({dt:.0f}s)")
print(f"  ffmpeg alone lzma: {len(ff_alone):,}")
print(f"  second copy (ffprobe) marginal cost under solid lzma: {len(both_lzma)-len(ff_alone):,} bytes")

print("\n=== whole-payload solid LZMA (what /SOLID lzma NSIS would give) ===")
payload = b''
total = 0
for f in FILES:
    p = os.path.join(STAGING, f)
    if os.path.exists(p):
        payload += open(p, 'rb').read()
        total += os.path.getsize(p)
t0 = time.time()
solid = lzma.compress(payload, preset=6)
dt = time.time() - t0
print(f"  payload {total:,} bytes -> solid lzma-6: {len(solid):,} bytes ({dt:.0f}s)")
print(f"  vs actual NSIS zlib installer: 147,768,031 bytes")
print(f"  saving: {147768031 - len(solid):,} bytes ({(147768031-len(solid))/1048576:.1f} MB)")

print("\n=== NSIS default zlib on same payload (sanity check vs actual installer) ===")
t0 = time.time()
z = zlib.compress(payload, 6)
print(f"  payload -> zlib-6 stream: {len(z):,} ({time.time()-t0:.0f}s) [NSIS uses its own zlib; same family]")
