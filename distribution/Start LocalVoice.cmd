@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\setup-portable.ps1" -Mode Start
if errorlevel 1 (
    echo.
    echo LocalVoice could not finish setup or start. Please read the message above.
    pause
    exit /b 1
)
