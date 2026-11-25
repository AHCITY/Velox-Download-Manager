#!/usr/bin/env python3
"""Fix the release notes block: printf-built notes file (no heredoc, no YAML breakage)."""

P = '/home/z/my-project/.github/workflows/windows.yml'
s = open(P).read()

start = s.index('        run: |\n          gh release create "$GITHUB_REF_NAME" out/* \\')
end = s.index('NOTES\n', start) + len('NOTES\n')
# find the true end: the heredoc terminator line 'NOTES' at column 0
end = s.index('\nNOTES', start) + len('\nNOTES') + 1

new_run = """        run: |
          VER="${GITHUB_REF_NAME#v}"
          {
            printf '%s\\n' "**Velox $VER** — themes, YouTube format selection, Linux support." ''
            printf '%s\\n' "### Windows" ''
            printf '%s\\n' "- **Velox-Setup-x64-unified.exe** — offline installer: app, browser extension + native messaging host, yt-dlp, LGPL ffmpeg/ffprobe (in \`bin/\`), WebView2 fallback. The in-app updater serves this file automatically." "- **latest.json** + signature — updater metadata (windows-x86_64)." ''
            printf '%s\\n' "### Linux x86_64 (per-user, no root)" ''
            printf '%s\\n' "- **Velox-linux-x64-$VER-offline.tar.gz** — OFFLINE: app + yt-dlp + LGPL FFmpeg bundled in \`bin/\`; extract and run \`install.sh\` (menu entry, launcher, icons included). Uninstall: \`./install.sh --uninstall\`." "- **Velox-linux-x64-$VER.tar.gz** — app core for the online installer." "- **velox-setup-linux-online.sh** — ONLINE installer: downloads the core + the YouTube toolchain from official sources (\`--no-media\` skips it)." ''
            printf '%s\\n' "Requirements: WebKitGTK 4.1 + GTK3 (present on current Ubuntu/Debian/Fedora/Arch). See docs/LINUX.md for install, update and troubleshooting." ''
            printf '%s\\n' "SHA256SUMS.txt covers every artifact. Downloads and settings live in the user profile on both platforms and are never touched by updates or uninstall."
          } > release-notes.md
          gh release create "$GITHUB_REF_NAME" out/* \\
            --title "Velox $VER" \\
            --notes-file release-notes.md
"""
s = s[:start] + new_run + s[end:]

open(P, 'w').write(s)
import yaml
yaml.safe_load(open(P))
print('release run block fixed; YAML valid')
