@echo off
:: Profile the loadtest bench using WPR (Windows Performance Recorder)
:: Usage: profile-bench.cmd [UNITS_PER_TEAM] [FRAMES]
:: Defaults: 500 units/team, 600 frames
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-bench.etl (open with WPA)

setlocal
set UNITS=%~1
set FRAMES=%~2
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600

cd /d "%~dp0.."

:: Set symbol path for WPA
set _NT_SYMBOL_PATH=%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building loadtest bench (profiling profile)...
cargo build --profile profiling --bench loadtest -p bar-game-lib --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

for %%f in (target\profiling\deps\loadtest-*.exe) do set "BENCH_EXE=%%f"

echo.
echo Starting WPR trace (CPU + GPU + DiskIO)...
wpr -start CPU -start GPU -start DiskIO
if %ERRORLEVEL% neq 0 (
    echo WPR failed. Run as Administrator.
    exit /b 1
)

echo.
echo Running loadtest: %UNITS% units/team, %FRAMES% frames...
echo When the bench finishes, the trace will be saved automatically.
echo.

:: Write a temporary stop script that runs in a clean cmd process
echo @echo off > "%TEMP%\wpr_stop.cmd"
echo wpr -stop "%CD%\profile-bench.etl" >> "%TEMP%\wpr_stop.cmd"

:: Run bench, then stop WPR in a fresh cmd
"%BENCH_EXE%" %UNITS% %FRAMES%

echo.
echo Stopping WPR trace (separate process)...
cmd /c "%TEMP%\wpr_stop.cmd"

if exist profile-bench.etl (
    echo.
    echo Trace saved to profile-bench.etl
    echo Opening in WPA...
    start "" profile-bench.etl
) else (
    echo.
    echo Trace save failed. Try manually:
    echo   wpr -stop profile-bench.etl
)
endlocal
