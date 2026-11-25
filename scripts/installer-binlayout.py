#!/usr/bin/env python3
"""bin/ layout for bundled tools across both NSIS templates + CI staging."""

U = '/home/z/my-project/scripts/installer-unified-template.nsi'
O = '/home/z/my-project/scripts/installer-online-template.nsi'
W = '/home/z/my-project/.github/workflows/windows.yml'

def edit(path, pairs):
    s = open(path).read()
    for old, new in pairs:
        assert old in s, f'{path}: pattern not found: {old[:70]!r}'
        s = s.replace(old, new, 1)
    open(path, 'w').write(s)
    print(f'{path}: {len(pairs)} edits')

# ---------- unified (offline) installer ----------
edit(U, [
    ("""  ; --- media tooling: bundled yt-dlp + LGPL ffmpeg (YouTube etc.) ---
  File "/oname=yt-dlp.exe" "${BUILD_STAGING}${SEP}yt-dlp.exe"
  File "/oname=ffmpeg.exe" "${BUILD_STAGING}${SEP}ffmpeg.exe"
  File "/oname=ffprobe.exe" "${BUILD_STAGING}${SEP}ffprobe.exe\"""",
     """  ; --- media tooling: bundled yt-dlp + LGPL ffmpeg in bin/ (organized,
      ;     identifiable, never mixed with app binaries) ---
  CreateDirectory "$INSTDIR\\bin"
  File "/oname=bin\\yt-dlp.exe" "${BUILD_STAGING}${SEP}bin${SEP}yt-dlp.exe"
  File "/oname=bin\\ffmpeg.exe" "${BUILD_STAGING}${SEP}bin${SEP}ffmpeg.exe"
  File "/oname=bin\\ffprobe.exe" "${BUILD_STAGING}${SEP}bin${SEP}ffprobe.exe\""""),
    ("""  IfFileExists "$INSTDIR\\yt-dlp.exe" +2 0""",
     """  IfFileExists "$INSTDIR\\bin\\yt-dlp.exe" +2 0"""),
    ("""  IfFileExists "$INSTDIR\\ffmpeg.exe" +2 0""",
     """  IfFileExists "$INSTDIR\\bin\\ffmpeg.exe" +2 0"""),
    ("""  IfFileExists "$INSTDIR\\ffprobe.exe" +2 0""",
     """  IfFileExists "$INSTDIR\\bin\\ffprobe.exe" +2 0"""),
    ("""  Delete "$INSTDIR\\yt-dlp.exe"
  Delete "$INSTDIR\\ffmpeg.exe"
  Delete "$INSTDIR\\ffprobe.exe\"""",
     """  Delete "$INSTDIR\\bin\\yt-dlp.exe"
  Delete "$INSTDIR\\bin\\ffmpeg.exe"
  Delete "$INSTDIR\\bin\\ffprobe.exe"
  RMDir "$INSTDIR\\bin"
  ; legacy flat layout (0.6.x in-place updates)
  Delete "$INSTDIR\\yt-dlp.exe"
  Delete "$INSTDIR\\ffmpeg.exe"
  Delete "$INSTDIR\\ffprobe.exe\""""),
])

# ---------- online installer ----------
edit(O, [
    ("""  IfFileExists "$INSTDIR\\yt-dlp.exe" 0 do_fetch
  IfFileExists "$INSTDIR\\ffmpeg.exe" 0 do_fetch
  IfFileExists "$INSTDIR\\ffprobe.exe" 0 do_fetch""",
     """  IfFileExists "$INSTDIR\\bin\\yt-dlp.exe" 0 do_fetch
  IfFileExists "$INSTDIR\\bin\\ffmpeg.exe" 0 do_fetch
  IfFileExists "$INSTDIR\\bin\\ffprobe.exe" 0 do_fetch"""),
    ("""  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\\velox-fetch.ps1" -DestDir "$INSTDIR"'""",
     """  CreateDirectory "$INSTDIR\\bin"
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\\velox-fetch.ps1" -DestDir "$INSTDIR\\bin"'"""),
    ("""  IfFileExists "$INSTDIR\\yt-dlp.exe" +2 0
    DetailPrint "note: yt-dlp.exe not present (component failed or skipped)"
  IfFileExists "$INSTDIR\\ffmpeg.exe" +2 0
    DetailPrint "note: ffmpeg.exe not present (component failed or skipped)\"""",
     """  IfFileExists "$INSTDIR\\bin\\yt-dlp.exe" +2 0
    DetailPrint "note: bin\\yt-dlp.exe not present (component failed or skipped)"
  IfFileExists "$INSTDIR\\bin\\ffmpeg.exe" +2 0
    DetailPrint "note: bin\\ffmpeg.exe not present (component failed or skipped)\""""),
    ("""  Delete "$INSTDIR\\yt-dlp.exe"
  Delete "$INSTDIR\\ffmpeg.exe"
  Delete "$INSTDIR\\ffprobe.exe\"""",
     """  Delete "$INSTDIR\\bin\\yt-dlp.exe"
  Delete "$INSTDIR\\bin\\ffmpeg.exe"
  Delete "$INSTDIR\\bin\\ffprobe.exe"
  RMDir "$INSTDIR\\bin"
  Delete "$INSTDIR\\yt-dlp.exe"
  Delete "$INSTDIR\\ffmpeg.exe"
  Delete "$INSTDIR\\ffprobe.exe\""""),
])

# ---------- CI: stage tools into bin/, strict verify ----------
s = open(W).read()
old_stage = """          # yt-dlp: official signed release binary
          Invoke-WebRequest -Uri "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$env:STAGING\\yt-dlp.exe\""""
new_stage = """          # yt-dlp: official signed release binary (0.7.0+ installs into bin/)
          New-Item -ItemType Directory -Force "$env:STAGING\\bin" | Out-Null
          Invoke-WebRequest -Uri "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" -OutFile "$env:STAGING\\bin\\yt-dlp.exe\""""
assert old_stage in s
s = s.replace(old_stage, new_stage, 1)
s = s.replace(
    '''Copy-Item (Get-ChildItem "$env:STAGING\\ffmpeg" -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1).FullName "$env:STAGING\\ffmpeg.exe"''',
    '''Copy-Item (Get-ChildItem "$env:STAGING\\ffmpeg" -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1).FullName "$env:STAGING\\bin\\ffmpeg.exe"''', 1)
s = s.replace(
    '''Copy-Item (Get-ChildItem "$env:STAGING\\ffmpeg" -Recurse -Filter "ffprobe.exe" | Select-Object -First 1).FullName "$env:STAGING\\ffprobe.exe"''',
    '''Copy-Item (Get-ChildItem "$env:STAGING\\ffmpeg" -Recurse -Filter "ffprobe.exe" | Select-Object -First 1).FullName "$env:STAGING\\bin\\ffprobe.exe"''', 1)
# strict verify: tools under bin/
s = s.replace(
    '''"Velox.exe", "velox-browser-host.exe", "yt-dlp.exe",
            "ffmpeg.exe", "ffprobe.exe", "extension", "manifest.json",''',
    '''"Velox.exe", "velox-browser-host.exe", "bin/yt-dlp.exe",
            "bin/ffmpeg.exe", "bin/ffprobe.exe", "extension", "manifest.json",''', 1)
open(W, 'w').write(s)
print('windows.yml: staging + verify updated for bin/ layout')
