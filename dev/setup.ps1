<#
.SYNOPSIS
  One-time setup of a Relay development environment on Windows.

.DESCRIPTION
  - Checks for Node.js, Rust, the Visual Studio C++ build tools and WebView2
  - Installs the npm packages for the app (mobile/) and the desktop shell
  - Adds Windows Firewall rules so the phone can reach the dev servers over
    Tailscale ONLY (100.64.0.0/10): Metro on 8081 and the mock server on 8766
  - Builds the mock llama-server used by `dev.ps1 -Mock`

  Run it from any PowerShell window: .\dev\setup.ps1
  It asks for administrator rights once, for the firewall rules.
#>
param([switch]$SkipFirewall)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Need($cmd, $hint) {
  if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
    Write-Host "Missing: $cmd. $hint" -ForegroundColor Red
    $script:missing = $true
  } else { Write-Host "Found $cmd" -ForegroundColor Green }
}

$missing = $false
Need node  'Install Node.js 22 LTS from https://nodejs.org'
Need npm   'It comes with Node.js'
Need cargo 'Install Rust from https://rustup.rs (choose the default MSVC toolchain)'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere) -or -not (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
  Write-Host 'Missing: Visual Studio C++ build tools. Install "Desktop development with C++" from https://visualstudio.microsoft.com/visual-cpp-build-tools/' -ForegroundColor Red
  $missing = $true
} else { Write-Host 'Found the C++ build tools' -ForegroundColor Green }
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
if (-not $wv) { Write-Host 'WebView2 not found (it ships with Windows 11; on Windows 10 get it from https://go.microsoft.com/fwlink/p/?LinkId=2124703)' -ForegroundColor Yellow }
if ($missing) { throw 'Install the missing tools above, then run this script again.' }

Write-Host "`nInstalling npm packages..." -ForegroundColor Cyan
Push-Location "$root\mobile";  npm install; Pop-Location
Push-Location "$root\desktop"; npm install; Pop-Location

Write-Host "`nBuilding the mock llama-server..." -ForegroundColor Cyan
Push-Location "$root\server"; cargo build --example mock_llama; Pop-Location

if (-not $SkipFirewall) {
  Write-Host "`nAdding firewall rules (Tailscale addresses only)..." -ForegroundColor Cyan
  $node = (Get-Command node).Source
  $rules = @"
netsh advfirewall firewall delete rule name="Relay dev: Metro (Tailscale only)" | Out-Null
netsh advfirewall firewall add rule name="Relay dev: Metro (Tailscale only)" dir=in action=allow protocol=TCP localport=8081 remoteip=100.64.0.0/10 program="$node" profile=any
netsh advfirewall firewall delete rule name="Relay dev: server (Tailscale only)" | Out-Null
netsh advfirewall firewall add rule name="Relay dev: server (Tailscale only)" dir=in action=allow protocol=TCP localport=8766 remoteip=100.64.0.0/10 profile=any
"@
  Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile', '-Command', $rules
  Write-Host 'If Windows asks whether Node.js may use the network, click Cancel: the rules above already allow Tailscale.' -ForegroundColor Yellow
}

Write-Host "`nDone. Start developing with: .\dev\dev.ps1 -Mock" -ForegroundColor Green
