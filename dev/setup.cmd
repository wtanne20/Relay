@echo off
rem One-time setup of the Relay dev environment. Double-click to run.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
pause
