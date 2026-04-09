@echo off
:: Generate flamegraph from the actual game binary (with rendering)
:: Usage: flamegraph-game.cmd [OUTPUT]
:: Defaults: flamegraph-game.svg
::
:: Requires: cargo-flamegraph, xperf (Windows Performance Toolkit)
:: Run as Administrator (xperf requires elevated privileges)
::
:: The game will launch normally. Play/observe for a while, then close
:: the window. The flamegraph is generated from the recorded samples.

setlocal
set OUTPUT=%~1
if "%OUTPUT%"=="" set OUTPUT=flamegraph-game.svg

echo Profiling game binary...
echo Output: %OUTPUT%
echo.
echo Play the game, then close the window to generate the flamegraph.
echo.

cd /d "%~dp0.."
cargo flamegraph --profile profiling --bin bar-game -p bar-game --features gpu-compute -o "%OUTPUT%"

if %ERRORLEVEL% equ 0 (
    echo.
    echo Flamegraph written to %OUTPUT%
    echo Opening in browser...
    start "" "%OUTPUT%"
) else (
    echo.
    echo Flamegraph generation failed.
    echo Make sure you run this as Administrator.
)
endlocal
