@echo off
:: Profile the game binary using WPR (Windows Performance Recorder)
:: Usage: profile-game.cmd [--loadtest] [--max-units N]
::
:: This script does NOT start/stop WPR itself to avoid COM conflicts.
:: Run wpr -start and wpr -stop manually in a separate admin terminal.
::
:: Steps:
::   1. Open an admin terminal (PowerShell or cmd)
::   2. Run: wpr -start CPU -start GPU -start DiskIO
::   3. Run this script: scripts\profile-game.cmd --loadtest
::   4. Close the game window
::   5. In the admin terminal run: wpr -stop profile-game.etl

setlocal enabledelayedexpansion
set GAME_ARGS=

:parse_args
if "%~1"=="" goto done_args
set "GAME_ARGS=!GAME_ARGS! %~1"
shift
goto parse_args
:done_args

cd /d "%~dp0.."

set _NT_SYMBOL_PATH=%CD%\target\profiling;%CD%\target\profiling\deps;%_NT_SYMBOL_PATH%

echo Building game (profiling profile)...
cargo build --profile profiling --bin bar-game -p bar-game --features gpu-compute
if %ERRORLEVEL% neq 0 (
    echo Build failed.
    exit /b 1
)

echo.
echo ============================================================
echo  Make sure WPR is recording!
echo  If not, run in a separate admin terminal:
echo    wpr -start CPU -start GPU -start DiskIO
echo ============================================================
echo.
pause

echo Launching game...
if defined GAME_ARGS (
    echo Game args:%GAME_ARGS%
)
target\profiling\bar-game.exe%GAME_ARGS%

echo.
echo ============================================================
echo  Game closed. Now stop WPR in the admin terminal:
echo    wpr -stop profile-game.etl
echo ============================================================
endlocal
