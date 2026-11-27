# Velox online-installer media fetcher — downloads the optional YouTube
# toolchain (yt-dlp + LGPL ffmpeg/ffprobe) from their OFFICIAL release
# sources at install time. Invoked by installer-online-template.nsi:
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File velox-fetch.ps1 [-DestDir <dir>]
#
# Exit codes: 0 = everything downloaded and extracted
#             2 = one or more tools could not be fetched (partial install —
#                 Velox itself keeps working, direct downloads unaffected)
#
# Sources (pinned to the maintainers' own "latest" release redirects):
#   yt-dlp   https://github.com/yt-dlp/yt-dlp  (Unlicense)
#   ffmpeg   https://github.com/BtbN/FFmpeg-Builds — win64 LGPL build
#            (no --enable-gpl; Velox's LGPL policy rejects GPL builds)
param(
    [string]$DestDir = $PSScriptRoot
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest renders a progress bar per chunk; on big files (the ~120 MB
# FFmpeg zip) that UI work can dominate the download and look like a mid-install
# hang. Silence it.
$ProgressPreference = 'SilentlyContinue'
# GitHub requires TLS 1.2+; older PowerShell defaults can try 1.0/1.1 first.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$YtDlpUrl   = 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe'
$FfmpegUrl  = 'https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-lgpl.zip'

function Get-RemoteFile {
    param([string]$Url, [string]$Dest, [int]$Tries = 3)
    for ($i = 1; $i -le $Tries; $i++) {
        try {
            Write-Host "downloading: $Url (attempt $i of $Tries)"
            Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
            if ((Test-Path $Dest) -and ((Get-Item $Dest).Length -gt 0)) { return $true }
            throw "download produced an empty file"
        }
        catch {
            Write-Host "attempt $i failed: $($_.Exception.Message)"
            if ($i -lt $Tries) { Start-Sleep -Seconds (2 * $i) }
        }
    }
    return $false
}

$ok = $true

# --- yt-dlp: single self-contained executable ---
if (-not (Get-RemoteFile -Url $YtDlpUrl -Dest (Join-Path $DestDir 'yt-dlp.exe'))) {
    $ok = $false
    Write-Host 'ERROR: yt-dlp could not be downloaded'
}

# --- ffmpeg: LGPL build ships as a zip; extract only the two tools + licence ---
$tmp = Join-Path $env:TEMP "velox-ffmpeg-$PID"
try {
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $zip = Join-Path $tmp 'ffmpeg.zip'
    if (Get-RemoteFile -Url $FfmpegUrl -Dest $zip) {
        Write-Host 'extracting ffmpeg archive...'
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $ffmpeg   = Get-ChildItem $tmp -Recurse -Filter 'ffmpeg.exe'  | Select-Object -First 1
        $ffprobe  = Get-ChildItem $tmp -Recurse -Filter 'ffprobe.exe' | Select-Object -First 1
        if ($ffmpeg -and $ffprobe) {
            Copy-Item $ffmpeg.FullName  (Join-Path $DestDir 'ffmpeg.exe')  -Force
            Copy-Item $ffprobe.FullName (Join-Path $DestDir 'ffprobe.exe') -Force
            $licence = Get-ChildItem $tmp -Recurse -Filter 'LICENSE*' | Select-Object -First 1
            if ($licence) {
                Copy-Item $licence.FullName (Join-Path $DestDir 'FFMPEG-LICENSE.txt') -Force
            }
            Write-Host "ffmpeg: $($ffmpeg.VersionInfo.FileVersion)"
        }
        else {
            $ok = $false
            Write-Host 'ERROR: ffmpeg.exe/ffprobe.exe not found inside the downloaded archive'
        }
    }
    else {
        $ok = $false
        Write-Host 'ERROR: ffmpeg archive could not be downloaded'
    }
}
finally {
    Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
}

if ($ok) {
    Write-Host 'YouTube toolchain ready: yt-dlp, ffmpeg, ffprobe (LGPL)'
    exit 0
}
exit 2
