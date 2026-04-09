@echo off
:: Parse Jira MCP JSON output into readable table
:: Usage: jira-parse.cmd <file>
if "%~1"=="" (echo Usage: jira-parse.cmd ^<file^> & exit /b 1)
powershell -NoProfile -Command "$r=(Get-Content -Raw '%~1'|ConvertFrom-Json)[0].text|ConvertFrom-Json; $r.issues|ForEach-Object{'{0} | {1} | {2} | {3} | {4}' -f $_.key,$_.fields.issuetype.name,$_.fields.status.name,$_.fields.priority.name,$_.fields.summary}"
