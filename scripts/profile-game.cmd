@echo off
:: Profile the game binary using WPR (Windows Performance Recorder)
:: Usage: profile-game.cmd [--loadtest] [--max-units N]
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-game.etl (open with WPA)

setlocal enabledelayedexpansion
set GAME_ARGS=

:parse_args
if "%~1"=="" goto done_args
set "GAME_ARGS=!GAME_ARGS! %~1"
shift
goto parse_args
:done_args

cd /d "%~dp0.."

:: Set symbol path for WPA
set _NT_SYMBOL_PATH=%CD%\target\profiling;%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building game (profiling profile)...
cargo build --profile profiling --bin bar-game -p bar-game --features gpu-compute
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
echo Launching game... Close the window to stop profiling.
if defined GAME_ARGS (
    echo Game args:%GAME_ARGS%
)

:: Write temporary stop script
echo @echo off > "%TEMP%\wpr_stop.cmd"
echo wpr -stop "%CD%\profile-game.etl" >> "%TEMP%\wpr_stop.cmd"

:: Run game
target\profiling\bar-game.exe%GAME_ARGS%

echo.
echo Stopping WPR trace (separate process)...
cmd /c "%TEMP%\wpr_stop.cmd"

if exist profile-game.etl (
    echo.
    echo Trace saved to profile-game.etl
    echo Opening in WPA...
    start "" profile-game.etl
) else (
    echo.
    echo Trace save failed. Try manually:
    echo   wpr -stop profile-game.etl
)
endlocal
