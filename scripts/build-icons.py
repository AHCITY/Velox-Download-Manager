#!/usr/bin/env python3
"""Generate download-engine/web/assets/icons.js from the iconoir + coolicons
sources (both MIT). Extracts inner SVG markup, strips svg-root-only attributes
(width/height/stroke-width/fill) that the central icon() renderer already
supplies, and emits one ICONS map entry per Velox icon name.

Handpicked mapping — iconoir (stroke-1.5 outline set) is the primary system;
coolicons fills the gaps it covers better (file types, folder-open,
external-link, keyboard, spinner, checkbox). Run again ONLY when re-picking
icons; /tmp/icons holds the shallow clones.
"""
import re
from pathlib import Path

ICONOIR = Path("/tmp/icons/iconoir/icons/regular")
COOL = Path("/tmp/icons/coolicons/icons/svg/line/regular")
OUT = Path("/home/z/my-project/download-engine/web/assets/icons.js")

# Velox name -> (collection, filename)
MAP = {
    "home":        ("iconoir", "home-simple.svg"),
    "download":    ("iconoir", "download.svg"),
    "list":        ("iconoir", "list.svg"),
    "queue":       ("iconoir", "playlist-play.svg"),
    "globe":       ("iconoir", "globe.svg"),
    "network":     ("iconoir", "wifi.svg"),
    "devices":     ("iconoir", "smartphone-device.svg"),
    "history":     ("iconoir", "clock-rotate-right.svg"),
    "vault":       ("iconoir", "safe.svg"),
    "settings":    ("iconoir", "settings.svg"),
    "search":      ("iconoir", "search.svg"),
    "plus":        ("iconoir", "plus.svg"),
    "x":           ("iconoir", "xmark.svg"),
    "pause":       ("iconoir", "pause.svg"),
    "play":        ("iconoir", "play.svg"),
    "retry":       ("iconoir", "refresh.svg"),
    "trash":       ("iconoir", "trash.svg"),
    "folder":      ("iconoir", "folder.svg"),
    "folderOpen":  ("cool",     "folder-open.svg"),
    "copy":        ("iconoir", "copy.svg"),
    "link":        ("iconoir", "link.svg"),
    "external":    ("cool",     "external-link.svg"),
    "check":       ("iconoir", "check.svg"),
    "checkCircle": ("iconoir", "check-circle.svg"),
    "alert":       ("iconoir", "warning-circle.svg"),
    "alertTriangle": ("iconoir", "warning-triangle.svg"),
    "info":        ("iconoir", "info-circle.svg"),
    "chevronDown": ("iconoir", "nav-arrow-down.svg"),
    "chevronRight": ("iconoir", "nav-arrow-right.svg"),
    "chevronUp":   ("iconoir", "nav-arrow-up.svg"),
    "chevronLeft": ("iconoir", "nav-arrow-left.svg"),
    "more":        ("iconoir", "more-horiz.svg"),
    "file":        ("iconoir", "page.svg"),
    "fileText":    ("cool",     "file-text.svg"),
    "archive":     ("cool",     "file-archive.svg"),
    "fileAudio":   ("iconoir", "music-double-note.svg"),
    "fileVideo":   ("cool",     "film.svg"),
    "fileImage":   ("cool",     "image.svg"),
    "fileCode":    ("cool",     "file-code.svg"),
    "package":     ("iconoir", "package.svg"),
    "drive":       ("iconoir", "google-drive.svg"),
    "activity":    ("iconoir", "activity.svg"),
    "zap":         ("iconoir", "flash.svg"),
    "bell":        ("iconoir", "bell.svg"),
    "keyboard":    ("cool",     "keyboard.svg"),
    "sliders":     ("iconoir", "control-slider.svg"),
    "filter":      ("iconoir", "filter.svg"),
    "arrowUp":     ("iconoir", "arrow-up.svg"),
    "arrowDown":   ("iconoir", "arrow-down.svg"),
    "shieldCheck": ("iconoir", "shield-check.svg"),
    "shieldX":     ("iconoir", "shield-xmark.svg"),
    "refresh":     ("iconoir", "refresh.svg"),
    "loader":      ("cool",     "spinner.svg"),
    "menu":        ("iconoir", "menu.svg"),
    "panelLeft":   ("iconoir", "sidebar-expand.svg"),
    "database":    ("iconoir", "database.svg"),
    "terminal":    ("iconoir", "terminal.svg"),
    "plug":        ("iconoir", "plug-type-c.svg"),
    "clock":       ("iconoir", "clock.svg"),
    "hash":        ("iconoir", "hashtag.svg"),
    "eye":         ("iconoir", "eye.svg"),
    "minus":       ("iconoir", "minus.svg"),
    "squareCheck": ("cool",     "square-check.svg"),
    "merge":       ("iconoir", "combine.svg"),
    "wifi":        ("iconoir", "wifi.svg"),
    "hardDrive":   ("iconoir", "hard-drive.svg"),
}

STRIP = re.compile(r'\s(?:width|height|stroke-width)="[^"]*"')


def inner(path: Path) -> str:
    svg = path.read_text()
    m = re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S)
    if not m:
        raise SystemExit(f"no svg body in {path}")
    body = m.group(1)
    body = STRIP.sub("", body)
    # collapse whitespace between elements for a compact bundle
    body = re.sub(r">\s+<", "><", body.strip())
    return body


def main() -> None:
    lines = [
        "// Velox icon set — curated subset of two MIT-licensed open-source",
        "// collections, drawn on the shared 24×24 outline grid:",
        "//   primary:  Iconoir (https://github.com/iconoir-icons/iconoir) — MIT",
        "//   gaps:     Coolicons (https://github.com/krystonschwarze/coolicons) — MIT",
        "// Inner path data only; the central icon() renderer supplies the svg",
        "// wrapper (stroke=currentColor, stroke-width=1.5, round caps/joins).",
        "// Regenerate with scripts/build-icons.py after re-picking.",
        "export const ICONS = {",
    ]
    missing = []
    for name in sorted(MAP):
        src, fname = MAP[name]
        path = (ICONOIR if src == "iconoir" else COOL) / fname
        if not path.exists():
            missing.append((name, src, fname))
            continue
        body = inner(path)
        lines.append(f"  {name}: '{body}',")
    lines += ["};", "", "export function icon(name, size = 16, cls = '') {",
              "  const body = ICONS[name] || ICONS.file;",
              "  return `<svg class=\"ic ${cls}\" width=\"${size}\" height=\"${size}\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">${body}</svg>`;",
              "}", ""]
    if missing:
        for name, src, fname in missing:
            print(f"MISSING: {name} -> {src}/{fname}")
        raise SystemExit(1)
    OUT.write_text("\n".join(lines))
    n = len(MAP)
    print(f"wrote {OUT} with {n} icons, {OUT.stat().st_size} bytes")


if __name__ == "__main__":
    main()
