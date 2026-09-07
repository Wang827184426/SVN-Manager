@echo off
rem Drag and drop the new .exe onto this file to publish it.
rem The publish target directory is resolved by update_server.py:
rem   1) update_root.txt (first non-empty line, next to this bat)
rem   2) update_root\ folder next to this bat
cd /d "%~dp0"
set "PY=python"
where py >nul 2>&1 && set "PY=py"
"%PY%" "%~dp0update_server.py" %*
