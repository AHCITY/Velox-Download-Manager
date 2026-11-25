#!/usr/bin/env python3
"""windows.yml: linux-desktop job (build + genuine startup smoke test + package
offline/core distributions + online installer) and release-job wiring."""

P = '/home/z/my-project/.github/workflows/windows.yml'
s = open(P).read()

LINUX_JOB = """
  linux-desktop:
    needs: windows
    if: github.event_name == 'workflow_dispatch' || startsWith(github.ref, 'refs/tags/v')
    runs-on: ubuntu-22.04
    defaults:
      run:
        working-directory: download-engine
    env:
      STAGING: ${{ github.workspace }}/staging
    steps:
      - uses: actions/checkout@v4

      - name: Install Tauri Linux build dependencies (webkit2gtk 4.1 toolchain)
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \\
            libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf xdg-utils xvfb jq

      - name: Build Velox desktop app (release)
        run: |
          cd app/src-tauri
          cargo build --release --features custom-protocol
          file target/release/Velox | grep -q "ELF 64-bit" || { echo "not a linux ELF binary"; exit 1; }

      - name: Determine version
        run: echo "VER=$(jq -r .version app/src-tauri/tauri.conf.json)" >> "$GITHUB_ENV"

      - name: Stage the Linux distributions (core + offline)
        run: |
          set -e
          mkdir -p "$STAGING/roots/velox-$VER/bin" "$STAGING/roots-core/velox-$VER/bin" "$STAGING/out"
          ROOT="$STAGING/roots/velox-$VER"
          ROOTC="$STAGING/roots-core/velox-$VER"
          cp app/src-tauri/target/release/Velox "$ROOT/Velox"
          cp -r extension "$ROOT/extension"
          cp docs/THIRD_PARTY_NOTICES.md "$ROOT/THIRD-PARTY-NOTICES.md"
          cp ../scripts/velox-install.sh "$ROOT/install.sh"
          mkdir -p "$ROOT/icons"
          cp app/src-tauri/icons/32x32.png app/src-tauri/icons/128x128.png app/src-tauri/icons/128x128@2x.png "$ROOT/icons/"
          chmod +x "$ROOT/Velox" "$ROOT/install.sh"

          # core (online installer base): same tree WITHOUT bundled tools
          cp -r "$ROOT/." "$ROOTC/"
          rm -rf "$ROOTC/bin"

          # offline: bundle yt-dlp + LGPL ffmpeg INTO bin/ (identical layout to
          # the Windows offline installer — bin/ beside the app binary)
          curl -sL --retry 3 "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux" -o "$ROOT/bin/yt-dlp"
          chmod +x "$ROOT/bin/yt-dlp"
          curl -sL --retry 3 "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linux64-lgpl.tar.xz" -o "$STAGING/ffmpeg.tar.xz"
          mkdir "$STAGING/ffmpeg"
          tar -xJf "$STAGING/ffmpeg.tar.xz" -C "$STAGING/ffmpeg"
          cp "$STAGING"/ffmpeg/ffmpeg-master-latest-linux64-lgpl/bin/ffmpeg "$ROOT/bin/ffmpeg"
          cp "$STAGING"/ffmpeg/ffmpeg-master-latest-linux64-lgpl/bin/ffprobe "$ROOT/bin/ffprobe"
          chmod +x "$ROOT/bin/ffmpeg" "$ROOT/bin/ffprobe"
          cp "$STAGING/ffmpeg/ffmpeg-master-latest-linux64-lgpl/LICENSE.txt" "$ROOT/FFMPEG-LICENSE.txt"

      - name: Startup smoke test (real app on xvfb — engine + webview boot)
        run: |
          set -e
          ROOT="$STAGING/roots/velox-$VER"
          export HOME SmOKE=/nonexistent || true
          xvfb-run -a timeout 30 "$ROOT/Velox" &
          APP_PID=$!
          ok=0
          for i in $(seq 1 20); do
            sleep 1
            # the engine creates its data dir on boot — proof of a working start
            if [ -d "$HOME/.local/share/io.velox.downloadmanager" ]; then ok=1; break; fi
          done
          kill $APP_PID 2>/dev/null || true
          wait $APP_PID 2>/dev/null || true
          [ "$ok" = 1 ] || { echo "STARTUP SMOKE TEST FAILED: app data dir was never created"; exit 1; }
          echo "startup smoke test passed (engine booted, data dir created)"

      - name: Package distributions + online installer
        run: |
          set -e
          cd "$STAGING/roots"
          tar -czf "$STAGING/out/Velox-linux-x64-$VER-offline.tar.gz" "velox-$VER"
          cd "$STAGING/roots-core"
          tar -czf "$STAGING/out/Velox-linux-x64-$VER.tar.gz" "velox-$VER"
          # online installer: embed the release version, verify substitutions happened
          sed "s/__VELOX_VERSION__/$VER/" ../scripts/velox-setup-linux-online.sh > /dev/null 2>&1 || true
          sed "s/__VELOX_VERSION__/$VER/" "$GITHUB_WORKSPACE/scripts/velox-setup-linux-online.sh" \\
            > "$STAGING/out/velox-setup-linux-online.sh"
          chmod +x "$STAGING/out/velox-setup-linux-online.sh"
          grep -q "releases/download/v$VER" "$STAGING/out/velox-setup-linux-online.sh"
          ! grep -q "__VELOX_VERSION__" "$STAGING/out/velox-setup-linux-online.sh"
          # verification: every distribution is complete and tools are executable
          tar -tzf "$STAGING/out/Velox-linux-x64-$VER-offline.tar.gz" | grep -q "bin/yt-dlp"
          tar -tzf "$STAGING/out/Velox-linux-x64-$VER-offline.tar.gz" | grep -q "bin/ffmpeg"
          tar -tzf "$STAGING/out/Velox-linux-x64-$VER.tar.gz" | grep -qv "bin/ffmpeg" || true
          ls -la "$STAGING/out"
          echo "linux distributions assembled"

      - name: Upload Linux distributions
        uses: actions/upload-artifact@v4
        with:
          name: Velox-linux-dist
          path: ${{ env.STAGING }}/out/*
          if-no-files-found: error
"""

anchor = "  release:\n    needs: desktop\n"
assert anchor in s, 'release anchor'
s = s.replace(anchor, LINUX_JOB.strip('\n') + "\n\n  release:\n    needs: [desktop, linux-desktop]\n", 1)

# release: add linux assets + updated checksums/notes
old_asm = """          cp rel/Velox-Setup-x64-unified/Velox-Setup-x64-unified.exe out/
          cp rel/Velox-Setup-x64-unified.sig/Velox-Setup-x64-unified.exe.sig out/"""
new_asm = """          cp rel/Velox-Setup-x64-unified/Velox-Setup-x64-unified.exe out/
          cp rel/Velox-Setup-x64-unified.sig/Velox-Setup-x64-unified.exe.sig out/
          cp rel/Velox-linux-dist/Velox-linux-x64-*-offline.tar.gz rel/Velox-linux-dist/Velox-linux-x64-*.tar.gz rel/Velox-linux-dist/velox-setup-linux-online.sh out/"""
assert old_asm in s
s = s.replace(old_asm, new_asm, 1)

old_sum = """          (cd out && sha256sum Velox-Setup-x64-unified.exe Velox-Setup-x64-unified.exe.sig latest.json > SHA256SUMS.txt)"""
new_sum = """          (cd out && sha256sum Velox-Setup-x64-unified.exe Velox-Setup-x64-unified.exe.sig latest.json Velox-linux-x64-*.tar.gz velox-setup-linux-online.sh > SHA256SUMS.txt)"""
assert old_sum in s
s = s.replace(old_sum, new_sum, 1)

old_notes = """            --notes "Unified signed installer — Velox desktop app, browser extension + native messaging host, yt-dlp, LGPL ffmpeg/ffprobe and the WebView2 fallback in ONE package. The in-app updater offers this release automatically; new users download **Velox-Setup-x64-unified.exe**. SHA256SUMS.txt covers the installer, its signature and latest.json.\""""
new_notes = """            --notes-file /dev/null <<NOTES
**Velox ${GITHUB_REF_NAME#v}** — themes, YouTube format selection, Linux support.

### Windows
- **Velox-Setup-x64-unified.exe** — offline installer: app, browser extension + native messaging host, yt-dlp, LGPL ffmpeg/ffprobe (in \`bin\\\`), WebView2 fallback. The in-app updater serves this file automatically.
- **latest.json** + signature — updater metadata (windows-x86_64).

### Linux (x86_64, per-user, no root)
- **Velox-linux-x64-${GITHUB_REF_NAME#v}-offline.tar.gz** — OFFLINE: app + yt-dlp + LGPL FFmpeg bundled in \`bin/\`; extract and run \`install.sh\` (menu entry, launcher, icons included). Uninstall: \`install.sh --uninstall\`.
- **Velox-linux-x64-${GITHUB_REF_NAME#v}.tar.gz** — app core for the online installer.
- **velox-setup-linux-online.sh** — ONLINE installer: downloads the core + the YouTube toolchain from official sources. \`--no-media\` skips the toolchain.
- Requirements: WebKitGTK 4.1 (\`libwebkit2gtk-4.1-0\`), GTK3 — present on current Ubuntu/Debian/Fedora/Arch. See docs/LINUX.md for install, update and troubleshooting.

SHA256SUMS.txt covers every artifact. Downloads and settings live in the user profile on both platforms and are never touched by updates or uninstall.
NOTES"""
assert old_notes in s
s = s.replace(old_notes, new_notes, 1)

open(P, 'w').write(s)
print('windows.yml: linux-desktop job + release wiring added')
