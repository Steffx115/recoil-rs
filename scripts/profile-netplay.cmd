@echo off
:: Profile a networked setup: 1 server + 2 headless clients, all on localhost.
:: Usage: profile-netplay.cmd [UNITS_PER_TEAM] [FRAMES]
:: Defaults: 500 units/team, 600 frames
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-netplay.etl (open with WPA - Windows Performance Analyzer)
::
:: The server runs on port 7878. Both headless clients connect to
:: 127.0.0.1:7878 and run for FRAMES ticks, then exit.
:: WPR captures all three processes in a single trace.

setlocal
set "WPR=C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\wpr.exe"
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

:: Set symbol path so WPA finds PDBs
set _NT_SYMBOL_PATH=%CD%\target\profiling;%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Cleaning old binaries...
del /q target\profiling\bar-server.exe target\profiling\bar-server.pdb 2>nul
del /q target\profiling\bar-headless-client.exe target\profiling\bar-headless-client.pdb 2>nul

echo Building server + headless client (profiling profile)...
cargo build --profile profiling -p bar-server --features gpu-compute --message-format=short
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

echo.
echo Starting WPR trace (CPU + DiskIO)...
"%WPR%" -start CPU -start DiskIO
if %ERRORLEVEL% neq 0 (
    echo WPR failed. Run as Administrator.
    exit /b 1
)

echo.
echo Starting server (port 7878, 2 players)...
start "Server" target\profiling\bar-server.exe --port 7878 --players-per-game 2

:: Give the server a moment to bind.
timeout /t 1 /nobreak >nul

echo Starting headless client 1 (%UNITS% units/team, %FRAMES% frames)...
start "Client 1" target\profiling\bar-headless-client.exe 127.0.0.1:7878 %UNITS% %FRAMES%

echo Starting headless client 2 (%UNITS% units/team, %FRAMES% frames)...
start "Client 2" target\profiling\bar-headless-client.exe 127.0.0.1:7878 %UNITS% %FRAMES%

echo.
echo Waiting for clients to finish...

:wait_clients
timeout /t 2 /nobreak >nul
tasklist /fi "imagename eq bar-headless-client.exe" 2>nul | find /i "bar-headless-client.exe" >nul
if %ERRORLEVEL% equ 0 goto wait_clients

echo.
echo Clients exited. Stopping server...
taskkill /im bar-server.exe /f >nul 2>&1

echo.
echo Stopping WPR trace...
"%WPR%" -stop profile-netplay.etl
if %ERRORLEVEL% equ 0 (
    echo.
    echo Trace saved to profile-netplay.etl
    echo Opening in WPA...
    start "" profile-netplay.etl
) else (
    echo Failed to save trace.
    "%WPR%" -cancel
)
endlocal
