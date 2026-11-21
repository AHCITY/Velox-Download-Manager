# Velox preview launcher for Windows (PowerShell).
# Starts the same stack as velox-preview.sh: a real local file server + the Velox
# engine/API/UI on http://localhost:3000. Ctrl-C stops both processes.
#
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\velox-preview.ps1
# Prereq: cargo build --release -p engine-cli -p engine-testserver --example throttled_httpd
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

$DataDir = Join-Path $Root "preview-data"
$UiDir   = Join-Path $Root "web"
$UiPort  = 3000
$FilePort = 8099
New-Item -ItemType Directory -Force -Path (Join-Path $DataDir "out") | Out-Null

$VeloxBin  = Join-Path $Root "target\release\velox.exe"
$HttpdBin  = Join-Path $Root "target\release\examples\throttled_httpd.exe"
if (!(Test-Path $VeloxBin) -or !(Test-Path $HttpdBin)) {
    Write-Host "release binaries missing — building (first run takes a few minutes)…"
    cargo build --release -p engine-cli -p engine-testserver --example throttled_httpd
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

function Test-PortBusy($port) {
    try {
        $c = New-Object Net.Sockets.TcpClient
        $c.Connect("127.0.0.1", $port); $c.Close(); return $true
    } catch { return $false }
}
if (Test-PortBusy $UiPort)   { Write-Error "port $UiPort is already in use (another preview running?)"; exit 1 }
if (Test-PortBusy $FilePort) { Write-Error "port $FilePort is already in use"; exit 1 }

$TestUrl = "http://127.0.0.1:$FilePort/file"
$procs = @()

# 1. real test file server: 96 MiB at ~12 MB/s per connection
$env:VELOX_NO_TESTSERVER = $env:VELOX_NO_TESTSERVER
if ($env:VELOX_NO_TESTSERVER -ne "1") {
    $procs += Start-Process -FilePath $HttpdBin -ArgumentList "96", "12000000", "0" `
        -WorkingDirectory $Root -PassThru -WindowStyle Hidden
}

# 2. engine + bridge + UI
$env:DXDL_HOME = $DataDir
$procs += Start-Process -FilePath $VeloxBin `
    -ArgumentList "serve-http", "--bind", "0.0.0.0", "--port", "$UiPort", `
                  "--ui-dir", "$UiDir", "--sample-url", "$TestUrl" `
    -WorkingDirectory $Root -PassThru -WindowStyle Hidden

Write-Host ""
Write-Host "─────────────────────────────────────────────────────────"
Write-Host " Velox preview"
Write-Host "   UI / API      : http://localhost:$UiPort"
Write-Host "   Demo download : $TestUrl  (96 MiB)"
Write-Host "   Data dir      : $DataDir (downloads in preview-data\out)"
Write-Host "   Stop          : close this PowerShell window or Ctrl-C"
Write-Host "─────────────────────────────────────────────────────────"

try { Wait-Process -Id $procs[-1].Id } finally {
    foreach ($p in $procs) { try { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } catch {} }
}
