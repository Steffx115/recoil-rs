@echo off
:: Profile the game binary using WPR (Windows Performance Recorder)
:: Usage: profile-game.cmd [--loadtest] [--max-units N]
::
:: Requires: WPR (Windows Performance Toolkit), run as Administrator
:: Output: profile-game.etl (open with WPA - Windows Performance Analyzer)
::
:: Examples:
::   profile-game.cmd                                Normal game
::   profile-game.cmd --loadtest                     Loadtest mode
::   profile-game.cmd --loadtest --max-units 5000    5000 units

setlocal enabledelayedexpansion
set GAME_ARGS=

:parse_args
if "%~1"=="" goto done_args
set "GAME_ARGS=!GAME_ARGS! %~1"
shift
goto parse_args
:done_args

cd /d "%~dp0.."

:: Set symbol path so WPA finds PDBs
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
:: Run in separate process to avoid COM threading conflict with WPR
start /wait "" target\profiling\bar-game.exe%GAME_ARGS%

echo.
echo Stopping WPR trace...
wpr -stop profile-game.etl
if %ERRORLEVEL% equ 0 (
    echo.
    echo Trace saved to profile-game.etl
    echo Opening in WPA...
    start "" profile-game.etl
) else (
    echo.
    echo WPR stop failed. Trying xperf fallback...
    xperf -stop
    xperf -d profile-game.etl
    if %ERRORLEVEL% equ 0 (
        echo Trace saved via xperf fallback.
        start "" profile-game.etl
    ) else (
        echo Failed to save trace. Run: wpr -cancel
    )
)
endlocal
