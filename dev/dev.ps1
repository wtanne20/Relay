<#
.SYNOPSIS
  Starts the Relay dev environment with live reload.

.DESCRIPTION
  Opens the Metro bundler in its own window, then the Relay desktop app in
  dev mode. Edit anything in mobile/src and it reloads in the desktop window
  and in the "Relay Dev" phone app within a second. Rust changes to the
  desktop shell or the server rebuild and restart the window automatically.

  -Mock      Use a throwaway server with a fake model (no GPU needed), on port
             8766 with its data in dev/data. Without it the desktop app runs the
             server from source with your real settings (%LOCALAPPDATA%\Relay),
             or connects to the installed Relay if that is already running.
  -NoDesktop Only start Metro (phone work).
  -NoPhone   Don't print the phone instructions.
  -Follow    Also pull changes pushed to this branch every few seconds, so
             they show up live (see follow.ps1).

.EXAMPLE
  .\dev\dev.ps1 -Mock
#>
param([switch]$Mock, [switch]$NoDesktop, [switch]$NoPhone, [switch]$Follow)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if ($Mock) {
  $data = Join-Path $PSScriptRoot 'data'
  New-Item -ItemType Directory -Force $data | Out-Null
  $cfg = Join-Path $data 'config.toml'
  if (-not (Test-Path $cfg)) {
    Push-Location "$root\server"; cargo build --example mock_llama; Pop-Location
    $mock = (Join-Path $root 'server\target\debug\examples\mock_llama.exe') -replace '\\', '/'
    $model = (Join-Path $data 'model.gguf') -replace '\\', '/'
    Set-Content -Path (Join-Path $data 'model.gguf') -Value 'GGUF'
    (Get-Content (Join-Path $PSScriptRoot 'mock-config.toml') -Raw).
      Replace('@MOCK_LLAMA@', $mock).Replace('@MODEL@', $model) | Set-Content $cfg
  }
  $env:RELAY_CONFIG = $cfg
  Write-Host "Mock mode: server on port 8766, data in $data" -ForegroundColor Cyan
}

# Metro must advertise an address the phone can reach: this PC's Tailscale IP.
$ts = $null
if (Get-Command tailscale -ErrorAction SilentlyContinue) { $ts = (tailscale ip -4 2>$null | Select-Object -First 1) }
if ($ts) { $env:REACT_NATIVE_PACKAGER_HOSTNAME = $ts }

Write-Host 'Starting Metro (live reload) in a new window...' -ForegroundColor Cyan
$metroEnv = "`$env:REACT_NATIVE_PACKAGER_HOSTNAME='$ts';"
Start-Process powershell -WorkingDirectory "$root\mobile" -ArgumentList '-NoExit', '-Command', "$metroEnv npx expo start --dev-client"

if ($Follow) {
  Write-Host 'Following the branch on GitHub in a new window...' -ForegroundColor Cyan
  Start-Process powershell -WorkingDirectory $root -ArgumentList '-NoExit', '-ExecutionPolicy', 'Bypass', '-File', "$PSScriptRoot\follow.ps1"
}

if (-not $NoPhone) {
  Write-Host ''
  Write-Host 'Phone: open the "Relay Dev" app (Relay-Dev.apk from the Android build) and' -ForegroundColor Green
  if ($ts) { Write-Host "  enter http://${ts}:8081 or scan the QR code in the Metro window." -ForegroundColor Green }
  else { Write-Host '  scan the QR code in the Metro window (Tailscale was not found, so it uses your LAN address).' -ForegroundColor Yellow }
  if ($Mock -and $ts) { Write-Host "  Inside the app, connect to http://${ts}:8766 with the token in dev\data\config.toml." -ForegroundColor Green }
  Write-Host ''
}

if (-not $NoDesktop) {
  Write-Host 'Starting the desktop app in dev mode (first build takes a few minutes)...' -ForegroundColor Cyan
  Push-Location "$root\desktop"
  try { npx tauri dev --additional-watch-folders "$root\server\src" } finally { Pop-Location }
}
