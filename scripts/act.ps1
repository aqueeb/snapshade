# scripts/act.ps1 — run nektos/act for this repo on Windows (docs/RELEASING.md, "Running the
# workflow locally with nektos/act"). Every argument is passed through to act, for example:
#   scripts\act.ps1 workflow_dispatch -W .github/workflows/release.yml -j windows --var-file .vars --secret-file .secrets
# What it fixes before calling act:
#   * act's host mode runs composite actions' `shell: bash` steps with the first `bash` on PATH,
#     which on this box is WSL's C:\Windows\System32\bash.exe — it cannot open C:\ paths and
#     fails with "/bin/bash: C:/Users/.../1-composite-parse.sh: No such file or directory".
#     Git for Windows' bash goes first.
#   * `shell: pwsh` steps need PowerShell 7 (winget install Microsoft.PowerShell); the MSIX build
#     is reachable as %LOCALAPPDATA%\Microsoft\WindowsApps\pwsh.exe.
#   * act itself is found through the WinGet links directory even in a shell started before
#     `winget install nektos.act`.
$ErrorActionPreference = "Stop"

$gitBin = Join-Path $env:ProgramFiles "Git\bin"
if (-not (Test-Path (Join-Path $gitBin "bash.exe"))) { throw "act.ps1: Git for Windows bash not found at $gitBin" }
$env:Path = "$gitBin;" + $env:Path

if (-not (Get-Command pwsh -ErrorAction SilentlyContinue)) {
  $alias = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps"
  if (Test-Path (Join-Path $alias "pwsh.exe")) { $env:Path = "$alias;" + $env:Path }
  else { throw "act.ps1: pwsh (PowerShell 7) not found: winget install Microsoft.PowerShell" }
}

$act = Get-Command act -ErrorAction SilentlyContinue
if (-not $act) {
  $link = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links\act.exe"
  if (Test-Path $link) { $act = Get-Command $link } else { throw "act.ps1: act not found: winget install nektos.act" }
}

Set-Location (Split-Path $PSScriptRoot -Parent)
# act logs to stderr; under Windows PowerShell 5.1 a redirected stderr line plus "Stop" would end
# the script with NativeCommandError before act does anything, so relax it for the call itself.
$ErrorActionPreference = "Continue"
& $act.Source @args
exit $LASTEXITCODE
