# Development launcher for DeepSeek Harness Desktop (Windows / PowerShell).
#
# Why this exists: the DeepSeek Harness Desktop installer puts its command shims
# in %LOCALAPPDATA%\deepseek-harness\bin and places that directory ahead of the
# npm global bin. Its `pnpm.cmd` refuses to run until the managed pnpm is
# installed, so `pnpm tauri dev` dies with "[pnpm] pnpm not found".
#
# This script fixes PATH for its own process only — nothing machine-wide changes.
#
# Usage:
#   .\scripts\dev.ps1                       # uses $env:OPENCODE_API_KEY
#   .\scripts\dev.ps1 -ApiKey oc_sk_...     # set the key for this run
#   .\scripts\dev.ps1 -Command build        # production build instead of dev

param(
  [string]$ApiKey = $env:OPENCODE_API_KEY,
  [ValidateSet('dev', 'build')]
  [string]$Command = 'dev'
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$shimDir = Join-Path $env:LOCALAPPDATA 'deepseek-harness\bin'
$npmDir = Join-Path $env:APPDATA 'npm'

if (-not (Test-Path (Join-Path $npmDir 'pnpm.cmd'))) {
  Write-Error "pnpm not found in $npmDir. Install it once with: npm i -g pnpm"
}

# Drop the shim directory from this process's PATH and put the real pnpm first.
$entries = $env:PATH -split ';' | Where-Object { $_ -and $_ -ne $shimDir }
$env:PATH = (@($npmDir) + $entries) -join ';'

if ($ApiKey) {
  $env:OPENCODE_API_KEY = $ApiKey
} else {
  Write-Warning 'OPENCODE_API_KEY is not set; the kernel will start but any prompt will fail.'
}

$env:DSH_DESKTOP_HOME = Join-Path $root '.dev-home'
$env:DSH_DESKTOP_KERNEL = Join-Path $root 'kernel'
$env:DSH_DESKTOP_BRIDGE = Join-Path $root 'bridge'
$env:RUST_LOG = 'dsh_desktop_app=debug,dsh_runtime=info,dsh_kernel=warn,info'

Write-Host "pnpm $(& pnpm --version) at $npmDir" -ForegroundColor DarkGray
Write-Host "DSH_HOME  $env:DSH_DESKTOP_HOME" -ForegroundColor DarkGray

Push-Location $root
try {
  if ($Command -eq 'build') {
    & pnpm build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & pnpm --filter '@dsh-desktop/app' tauri build
  } else {
    & pnpm --filter '@dsh-desktop/app' tauri dev
  }
  exit $LASTEXITCODE
} finally {
  Pop-Location
}
