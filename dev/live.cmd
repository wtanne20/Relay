@echo off
rem Live dev: Metro, the desktop app in dev mode, and auto-pull of pushed changes.
rem Uses your real Relay settings. Add -Mock to use a fake model instead.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0dev.ps1" -Follow %*
pause
