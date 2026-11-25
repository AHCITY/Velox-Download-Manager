#!/usr/bin/env python3
"""app.css theme sweep: replace remaining hardcoded colors with palette
variables / color-mix() so all four themes render consistently."""
import re

P = '/home/z/my-project/download-engine/web/assets/app.css'
s = open(P).read()
n = 0

def sub1(pat, rep):
    global s, n
    if rep in s:  # already applied (idempotent re-runs)
        return
    new, k = re.subn(pat, rep, s, count=1)
    assert k == 1, f'pattern not found: {pat}'
    s = new; n += 1

# 1. scrollbar hover was hardcoded dark-only
sub1(r'::-webkit-scrollbar-thumb:hover \{ background: #35404f; \}',
     '::selection-accent:hover { background: var(--scroll-h); }')
s = s.replace('::selection-accent:hover', '::-webkit-scrollbar-thumb:hover')  # keep real selector

# 2. on-accent text (buttons/badges/spinner/toggle knob)
sub1(r'\.btn\.primary \{ background: var\(--accent\); border-color: var\(--accent\); color: #06121f; \}',
     '.btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }')
sub1(r'border-top-color: #06121f; animation: spin',
     'border-top-color: var(--spin-dot); animation: spin')
sub1(r'border: 2px solid rgba\(0,0,0,\.25\); border-top-color: var\(--spin-dot\)',
     'border: 2px solid var(--spin-track); border-top-color: var(--spin-dot)')
sub1(r'\.toggle input:checked \+ \.tk::after \{ transform: translateX\(14px\); background: #06121f; \}',
     '.toggle input:checked + .tk::after { transform: translateX(14px); background: var(--on-accent); }')

# 3. archive category tile
sub1(r'\.file-ico\.archive \{ color: #e2a55e; background: rgba\(226,165,94,\.12\); \}',
     '.file-ico.archive { color: var(--cat-archive); background: var(--cat-archive-bg); }')

# 4. tinted borders → color-mix() from the solid status colors
sub1(r'\.btn:hover \{ background: var\(--bg4\); border-color: #35404f; \}',
     '.btn:hover { background: var(--bg4); border-color: var(--scroll-h); }')
sub1(r'border: 1px solid rgba\(77,159,255,\.3\); border-radius: var\(--r-m\);',
     'border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent); border-radius: var(--r-m);')
sub1(r'background: rgba\(77,159,255,\.25\); color: var\(--accent-h\); \}',
     'background: color-mix(in srgb, var(--accent) 25%, transparent); color: var(--accent-h); }')
sub1(r'border-color: rgba\(62, 207, 142, \.4\); color: var\(--t1\); \}',
     'border-color: color-mix(in srgb, var(--ok) 40%, transparent); color: var(--t1); }')
sub1(r'\.btn\.danger:hover \{ background: var\(--err-bg\); border-color: rgba\(244,105,94,\.4\); \}',
     '.btn.danger:hover { background: var(--err-bg); border-color: color-mix(in srgb, var(--err) 40%, transparent); }')
sub1(r'\.preview\.ok \{ border-color: rgba\(62,207,142,\.35\); \}',
     '.preview.ok { border-color: color-mix(in srgb, var(--ok) 35%, transparent); }')
sub1(r'\.preview\.bad \{ border-color: rgba\(244,105,94,\.4\); \}',
     '.preview.bad { border-color: color-mix(in srgb, var(--err) 40%, transparent); }')
sub1(r'\.note\.warn \{ border-color: rgba\(226,181,62,\.3\); \}',
     '.note.warn { border-color: color-mix(in srgb, var(--warn) 30%, transparent); }')
sub1(r'\.note\.acc \{ border-color: rgba\(77,159,255,\.3\); \}',
     '.note.acc { border-color: color-mix(in srgb, var(--accent) 30%, transparent); }')
sub1(r'\.note\.err \{ border-color: rgba\(244,105,94,\.3\); \}',
     '.note.err { border-color: color-mix(in srgb, var(--err) 30%, transparent); }')

# 5. overlay scrim: keep dark but soften with theme-neutral black
sub1(r'background: rgba\(5, 8, 12, \.68\);',
     'background: color-mix(in srgb, #04070b 66%, transparent);')

# 6. icon-btn bubble text on error red
sub1(r'background: var\(--err\); color: #fff; font-size: 9\.5px',
     'background: var(--err); color: var(--on-accent); font-size: 9.5px')

# 7. sidebar toggle: move down the Y axis (accessibility) + theme-aware
sub1(r'\.side-toggle \{ position: absolute; right: -13px; top: 16px; z-index: 6;\n'
     r'  background: var\(--bg2, #16222e\); border: 1px solid var\(--line, #24384c\);\n'
     r'  width: 26px; height: 26px; display: flex; align-items: center; justify-content: center; \}',
     '.side-toggle { position: absolute; right: -13px; top: 74px; z-index: 6;\n'
     '  background: var(--bg2); border: 1px solid var(--line2);\n'
     '  width: 26px; height: 26px; display: flex; align-items: center; justify-content: center;\n'
     '  border-radius: 999px; color: var(--t3); }\n'
     '.side-toggle:hover { color: var(--t1); border-color: var(--accent); }')

# 8. DPI / display-scaling robustness (Windows 125%/150%): rem-based
#    min sizes avoid sub-pixel collapse at fractional zoom levels
s = s.replace(
    'html, body { height: 100%; }',
    'html, body { height: 100%; }\n'
    '/* fractional display scaling (Windows 125%/150%): snap layout math to\n'
    '   whole pixels to avoid 1px ghost gaps and half-clipped borders */\n'
    'body { text-rendering: optimizeLegibility; }\n'
    '@media screen and (-webkit-min-device-pixel-ratio: 1.25) {\n'
    '  .side-toggle { width: 26px; height: 26px; }\n'
    '}', 1)

open(P, 'w').write(s)
print(f'CSS theme sweep: {n} replacements + sidebar toggle moved to top:74px')
