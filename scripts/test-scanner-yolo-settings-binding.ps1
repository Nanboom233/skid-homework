param(
  [switch]$SkipFrontendBuild,
  [switch]$SkipProbe
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Invoke-Step {
  param(
    [Parameter(Mandatory = $true)]
    [scriptblock]$Script
  )

  & $Script
  if ($LASTEXITCODE -ne 0) {
    throw "Native command failed with exit code $LASTEXITCODE."
  }
}

Write-Host "[1/5] TypeScript type check"
Invoke-Step { pnpm exec tsc --noEmit --pretty false }

if (-not $SkipFrontendBuild) {
  Write-Host "[2/5] Frontend build"
  Invoke-Step { pnpm build }
} else {
  Write-Host "[2/5] Frontend build skipped"
}

Write-Host "[3/5] Rust cargo check"
Invoke-Step { cargo check --manifest-path src-tauri/Cargo.toml }

Write-Host "[4/5] Rust cargo test"
Invoke-Step { cargo test --manifest-path src-tauri/Cargo.toml }

if (-not $SkipProbe) {
  Write-Host "[5/5] Native YOLO probe smoke"
  Invoke-Step { cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe }
} else {
  Write-Host "[5/5] Native YOLO probe skipped"
}
