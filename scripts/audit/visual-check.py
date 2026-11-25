#!/usr/bin/env python3
"""Programmatic visual check of the rendered report pages (image input unavailable):
- per-page non-white ratio (blank / nearly-blank page detection)
- text extraction order + key content presence per page
"""
import subprocess, sys
from PIL import Image
import glob

pages = sorted(glob.glob('/tmp/render/page-*.jpg'))
print(f"pages rendered: {len(pages)}")
bad = []
for p in pages:
    im = Image.open(p).convert('L')
    h = im.histogram()
    nonwhite = 1 - h[255] / sum(h)
    n = p.split('-')[-1].split('.')[0]
    flag = ''
    if int(n) == 1 and nonwhite > 0.9:
        flag = ' (R1 dark cover - by design)'
    elif nonwhite < 0.005:
        flag = ' <-- NEARLY BLANK'
        bad.append(p)
    elif int(n) != 1 and nonwhite > 0.55:
        flag = ' <-- suspiciously dense (overflow?)'
        bad.append(p)
    print(f"  page {n}: {im.size}, non-white {nonwhite:.3f}{flag}")

# text order check
txt = subprocess.run(['pdftotext', '/home/z/my-project/download/Velox-Pipeline-Installer-Audit.pdf', '-'],
                     capture_output=True, text=True).stdout
checks = [
    ('cover title', 'Velox Build Pipeline'),
    ('TOC', 'Table of Contents'),
    ('exec summary', 'Executive Summary'),
    ('benchmark table', 'cargo fetch --locked'),
    ('payload table', '138,752,000'),
    ('A/B table', '117,242,359'),
    ('alternatives', 'Inno Setup'),
    ('recommendation', 'Final Recommendation'),
    ('priority table', 'SetCompressor /SOLID lzma'),
    ('closing', 'Verification status'),
]
for name, needle in checks:
    print(f"  [{'OK' if needle in txt else 'MISSING'}] {name}")
# forbidden residues
for residue in ['XXX', 'TODO', 'TBD', 'undefined', 'NaN', '【']:
    if residue in txt:
        print(f"  [RESIDUE] {residue!r} found")
        bad.append(residue)
print('RESULT:', 'PASS' if not bad else f'FAIL {bad}')
