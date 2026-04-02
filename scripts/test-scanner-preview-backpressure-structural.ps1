param(
  [int]$DurationMs = 8000,
  [int]$Fps = 30,
  [int]$DownstreamMs = 36,
  [string]$BenchmarkJson = "outputs/runtime/vibe-sessions/20260403-scanner-preview-backpressure-001/benchmark-preview-backpressure.json",
  [string]$BenchmarkMarkdown = "docs/zh/scanner/2026-04-03-scanner-preview-backpressure-benchmark.md"
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Invoke-Step {
  param(
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][scriptblock]$Action
  )

  Write-Host ""
  Write-Host "==> $Name" -ForegroundColor Cyan
  & $Action
  if ($LASTEXITCODE -ne 0) {
    throw "Native command failed with exit code $LASTEXITCODE."
  }
}

Invoke-Step -Name "TypeScript typecheck" -Action {
  pnpm exec tsc --noEmit --pretty false
}

Invoke-Step -Name "Frontend production build" -Action {
  pnpm build
}

Invoke-Step -Name "Synthetic preview backpressure benchmark" -Action {
  node .\.tmp\scanner-preview-backpressure-benchmark.mjs --duration-ms $DurationMs --fps $Fps --downstream-ms $DownstreamMs --json $BenchmarkJson --md $BenchmarkMarkdown
}

Invoke-Step -Name "Git diff whitespace check" -Action {
  git diff --check
}
