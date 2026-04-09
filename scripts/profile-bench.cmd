@echo off
:: Profile the loadtest bench using WPR (Windows Performance Recorder)
:: Usage: profile-bench.cmd [UNITS_PER_TEAM] [FRAMES]
:: Defaults: 500 units/team, 600 frames
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-bench.etl (open with WPA - Windows Performance Analyzer)

setlocal
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

:: Set symbol path so WPA finds PDBs next to the exe
set _NT_SYMBOL_PATH=%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building loadtest bench (profiling profile)...
cargo build --profile profiling --bench loadtest -p bar-game-lib --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

echo.
echo Starting WPR trace (CPU + GPU + DiskIO)...
wpr -start CPU -start GPU -start DiskIO
if %ERRORLEVEL% neq 0 (
    echo WPR failed. Run as Administrator.
    exit /b 1
)

echo.
echo Running loadtest: %UNITS% units/team, %FRAMES% frames...
for %%f in (target\profiling\deps\loadtest-*.exe) do set "BENCH_EXE=%%f"

:: Run in a separate process to avoid COM threading conflict with WPR
start /wait "" "%BENCH_EXE%" %UNITS% %FRAMES%

echo.
echo Stopping WPR trace...
wpr -stop profile-bench.etl
if %ERRORLEVEL% equ 0 (
    echo.
    echo Trace saved to profile-bench.etl
    echo Symbol path: %_NT_SYMBOL_PATH%
    echo Opening in WPA...
    start "" profile-bench.etl
) else (
    echo.
    echo WPR stop failed. Trying xperf fallback...
    xperf -stop
    xperf -d profile-bench.etl
    if %ERRORLEVEL% equ 0 (
        echo Trace saved via xperf fallback.
        start "" profile-bench.etl
    ) else (
        echo Failed to save trace. Run: wpr -cancel
    )
)
endlocal
