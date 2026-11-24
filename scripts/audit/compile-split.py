#!/usr/bin/env python3
"""Audit: dependency-vs-app compile split from GitHub Actions desktop-job logs."""
import re
import sys
from datetime import datetime

LOGS = [
    ('scripts/audit/desktop-job-old.log', 'run1 (28799af4: makensis missing)'),
    ('scripts/audit/desktop-job-run2.log', 'run2 (a816ebf3: template path)'),
]

ANSI = re.compile(r'\x1b\[[0-9;]*m')

def parse(logfile):
    comp = []
    finished = []
    for ln in open(logfile, encoding='utf-8', errors='replace'):
        ln = ln.rstrip('\r\n')
        m = re.match(r'^([\d-]+T[\d:.]+Z)\s+(.*)$', ln)
        if not m:
            continue
        rest = ANSI.sub('', m.group(2)).replace('\r', ' ')
        ts = datetime.fromisoformat(m.group(1).replace('Z', '+00:00'))
        mm = re.search(r'Compiling ([\w-]+) v([\d.]+)', rest)
        if mm:
            comp.append((ts, mm.group(1)))
        if 'Finished' in rest:
            finished.append((ts, rest.strip()))
    return comp, finished

for logfile, label in LOGS:
    try:
        comp, finished = parse(logfile)
    except FileNotFoundError:
        print(f"{label}: log missing")
        continue
    if not comp:
        print(f"{label}: no Compiling lines found")
        continue
    total = (comp[-1][0] - comp[0][0]).total_seconds()
    print(f"\n{label}: {len(comp)} crates, compile span {total:.0f}s")
    app_idx = [i for i, (t, n) in enumerate(comp) if n == 'velox-app']
    if app_idx:
        i = app_idx[0]
        dep_s = (comp[i][0] - comp[0][0]).total_seconds()
        app_s = (comp[-1][0] - comp[i][0]).total_seconds()
        print(f"  deps ({comp[0][1]} -> velox-app): {dep_s:.0f}s for {i} crates")
        print(f"  velox-app itself (incl. fat-LTO + codegen-units=1 link): {app_s:.0f}s")
    gaps = sorted([((b[0] - a[0]).total_seconds(), a[1]) for a, b in zip(comp, comp[1:])])
    print("  slowest crates (upper bound): " + ", ".join(f"{n} {g:.0f}s" for g, n in gaps[-8:][::-1]))
    n_tauri = sum(1 for _, n in comp if 'tauri' in n or n in ('wry', 'wasm-bindgen', 'windows', 'windows-sys'))
    print(f"  tauri/wry/windows-family crates: {n_tauri}")
    for ts, msg in finished:
        print(f"  Finished @ {ts.strftime('%H:%M:%S')}: {msg[:80]}")
