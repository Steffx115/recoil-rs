@echo off
:: Generate flamegraph from the game binary
:: Usage: flamegraph-game.cmd [--loadtest] [OUTPUT]
::
:: Options:
::   --loadtest              Enable loadtest mode (auto-spawns waves of units)
::   --wave-size N           Units per wave (default: 50)
::   --max-units N           Max total units (default: 2000)
::
:: Requires: cargo-flamegraph, xperf (Windows Performance Toolkit)
:: Run as Administrator
::
:: Examples:
::   flamegraph-game.cmd                                Normal game
::   flamegraph-game.cmd --loadtest                     Loadtest, 2000 units
::   flamegraph-game.cmd --loadtest --max-units 5000    Loadtest, 5000 units

setlocal enabledelayedexpansion
set OUTPUT=flamegraph-game.svg
set GAME_ARGS=

:parse_args
if "%~1"=="" goto done_args
if "%~1"=="--loadtest" (
    set "GAME_ARGS=!GAME_ARGS! --loadtest"
    shift
    goto parse_args
)
if "%~1"=="--wave-size" (
    set "GAME_ARGS=!GAME_ARGS! --wave-size %~2"
    shift
    shift
    goto parse_args
)
if "%~1"=="--max-units" (
    set "GAME_ARGS=!GAME_ARGS! --max-units %~2"
    shift
    shift
    goto parse_args
)
:: Assume it's the output filename
set OUTPUT=%~1
shift
goto parse_args
:done_args

echo Profiling game binary...
echo Output: %OUTPUT%
if defined GAME_ARGS echo Game args:%GAME_ARGS%
echo.
echo Close the game window to generate the flamegraph.
echo.

cd /d "%~dp0.."
cargo flamegraph --profile profiling --bin bar-game -p bar-game --features gpu-compute -o "%OUTPUT%" --%GAME_ARGS%

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
