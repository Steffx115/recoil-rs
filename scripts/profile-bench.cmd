@echo off
:: Profile the loadtest bench using WPR (Windows Performance Recorder)
:: Usage: profile-bench.cmd [UNITS_PER_TEAM] [FRAMES]
::
:: This script does NOT start/stop WPR itself to avoid COM conflicts.
:: Run wpr -start and wpr -stop manually in a separate admin terminal.
::
:: Steps:
::   1. Open an admin terminal (PowerShell or cmd)
::   2. Run: wpr -start CPU -start GPU -start DiskIO
::   3. Run this script: scripts\profile-bench.cmd 5000 100
::   4. After it finishes, in the admin terminal run: wpr -stop profile-bench.etl

setlocal
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

set _NT_SYMBOL_PATH=%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building loadtest bench (profiling profile)...
cargo build --profile profiling --bench loadtest -p bar-game-lib --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

for %%f in (target\profiling\deps\loadtest-*.exe) do set "BENCH_EXE=%%f"

echo.
echo ============================================================
echo  Make sure WPR is recording!
echo  If not, run in a separate admin terminal:
echo    wpr -start CPU -start GPU -start DiskIO
echo ============================================================
echo.
pause

echo Running loadtest: %UNITS% units/team, %FRAMES% frames...
"%BENCH_EXE%" %UNITS% %FRAMES%

echo.
echo ============================================================
echo  Bench finished. Now stop WPR in the admin terminal:
echo    wpr -stop profile-bench.etl
echo ============================================================
endlocal
