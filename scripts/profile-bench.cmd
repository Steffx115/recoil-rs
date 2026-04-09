@echo off
:: Profile the loadtest bench using WPR (Windows Performance Recorder)
:: Usage: profile-bench.cmd [UNITS_PER_TEAM] [FRAMES]
:: Defaults: 500 units/team, 600 frames
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-bench.etl (open with WPA - Windows Performance Analyzer)
::
:: Examples:
::   profile-bench.cmd                    500 units, 600 frames
::   profile-bench.cmd 2000 200           2000 units, 200 frames

setlocal
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

:: Set symbol path so WPA finds PDBs
set _NT_SYMBOL_PATH=%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building loadtest bench (profiling profile)...
cargo build --profile profiling --bench loadtest -p bar-game-lib --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

echo.
echo Starting WPR trace (CPU sampling + context switches)...
wpr -start CPU -start GPU
if %ERRORLEVEL% neq 0 (
    echo WPR failed. Run as Administrator.
    exit /b 1
)

echo.
echo Finding latest loadtest binary...
:: Pick the newest exe (last modified)
for /f "delims=" %%f in ('dir /b /o:-d target\profiling\deps\loadtest-*.exe 2^>nul') do (
    set "BENCH_EXE=target\profiling\deps\%%f"
    goto :found_exe
)
echo No loadtest binary found.
exit /b 1
:found_exe
echo Running %BENCH_EXE%: %UNITS% units/team, %FRAMES% frames...
"%BENCH_EXE%" %UNITS% %FRAMES%

echo.
echo Stopping WPR trace...
wpr -stop profile-bench.etl
if %ERRORLEVEL% equ 0 (
    echo.
    echo Trace saved to profile-bench.etl
    echo Opening in WPA...
    start "" profile-bench.etl
) else (
    echo Failed to save trace.
    wpr -cancel
)
endlocal
