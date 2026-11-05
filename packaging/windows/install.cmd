@echo off
rem Install Aptechka (double-click). Remove: install.cmd -Uninstall
chcp 65001 >nul
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if "%1"=="" pause
