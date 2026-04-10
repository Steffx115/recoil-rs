@echo off
:: Profile the loadtest bench using WPR (Windows Performance Recorder)
:: Usage: profile-bench.cmd [UNITS_PER_TEAM] [FRAMES]
:: Defaults: 500 units/team, 600 frames
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-bench.etl (open with WPA - Windows Performance Analyzer)

setlocal
set "WPR=C:\Program Files (x86)\Windows Kits\10\Windows Performance Toolkit\wpr.exe"
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

:: Set symbol path so WPA finds PDBs
set _NT_SYMBOL_PATH=%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Cleaning old binaries...
del /q target\profiling\deps\loadtest-*.exe target\profiling\deps\loadtest-*.pdb 2>nul

echo Building loadtest bench (profiling profile)...
cargo build --profile profiling --bench loadtest -p bar-game-lib --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

echo.
echo Starting WPR trace (CPU sampling + context switches)...
"%WPR%" -start CPU -start GPU -start DiskIO
if %ERRORLEVEL% neq 0 (
    echo WPR failed. Run as Administrator.
    exit /b 1
)

echo.
echo Running loadtest: %UNITS% units/team, %FRAMES% frames...
set "BENCH_EXE="
for %%f in (target\profiling\deps\loadtest-*.exe) do set "BENCH_EXE=%%f"
if not defined BENCH_EXE (
    echo No loadtest binary found.
    "%WPR%" -cancel
    exit /b 1
)
"%BENCH_EXE%" %UNITS% %FRAMES%

echo.
echo Stopping WPR trace...
"%WPR%" -stop profile-bench.etl
if %ERRORLEVEL% equ 0 (
    echo.
    echo Trace saved to profile-bench.etl
    echo Opening in WPA...
    start "" profile-bench.etl
) else (
    echo Failed to save trace.
    "%WPR%" -cancel
)
endlocal
