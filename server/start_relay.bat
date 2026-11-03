@echo off
REM Starts Relay from this folder (config.toml, relay.db, logs\ and skills\ live here).
REM Build first with: cargo build --release
title Relay
cd /d "%~dp0"
if not exist "target\release\relay.exe" (
  echo relay.exe not found. Run "cargo build --release" in %~dp0 first.
  pause
  exit /b 1
)
"target\release\relay.exe" serve --config "%~dp0config.toml"
if errorlevel 1 pause
