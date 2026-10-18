@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\setup-portable.ps1" -Mode Menu
if errorlevel 1 (
    echo.
    echo Model setup did not finish. You can run this file again to retry.
    pause
    exit /b 1
)
pause
