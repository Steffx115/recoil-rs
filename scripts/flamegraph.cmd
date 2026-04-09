@echo off
:: Generate flamegraph from load test
:: Usage: flamegraph.cmd [UNITS_PER_TEAM] [FRAMES] [OUTPUT]
:: Defaults: 500 units/team, 600 frames, flamegraph.svg
::
:: Requires: cargo-flamegraph, xperf (Windows Performance Toolkit)
:: Install: cargo install flamegraph
::
:: Examples:
::   flamegraph.cmd                    500 units, 600 frames
::   flamegraph.cmd 1000 1000          1000 units, 1000 frames
::   flamegraph.cmd 200 300 quick.svg  200 units, 300 frames, custom output

setlocal
set UNITS=%~1
set FRAMES=%~2
set OUTPUT=%~3
if "%UNITS%"=="" set UNITS=500
if "%FRAMES%"=="" set FRAMES=600
if "%OUTPUT%"=="" set OUTPUT=flamegraph.svg

echo Load test: %UNITS% units/team, %FRAMES% frames
echo Output: %OUTPUT%

cd /d "%~dp0.."
cargo flamegraph --bench loadtest -p bar-game-lib -o "%OUTPUT%" -- %UNITS% %FRAMES%

if %ERRORLEVEL% equ 0 (
    echo.
    echo Flamegraph written to %OUTPUT%
    echo Opening in browser...
    start "" "%OUTPUT%"
) else (
    echo.
    echo Flamegraph generation failed.
    echo Make sure you run this as Administrator ^(xperf requires elevated privileges^).
)
endlocal
