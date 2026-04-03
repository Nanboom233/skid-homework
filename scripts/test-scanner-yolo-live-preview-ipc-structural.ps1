param(
  [string]$SmokeImagePath = ".github/images/skidhw-thumbnail-new.png"
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

Write-Host "[1/7] TypeScript type check"
Invoke-Step { pnpm exec tsc --noEmit --pretty false }

Write-Host "[2/7] Frontend build"
Invoke-Step { pnpm build }

Write-Host "[3/7] Rust cargo check"
Invoke-Step { cargo check --manifest-path src-tauri/Cargo.toml }

Write-Host "[4/7] Rust cargo test"
Invoke-Step { cargo test --manifest-path src-tauri/Cargo.toml }

Write-Host "[5/7] Rust cargo build smoke binary"
Invoke-Step { cargo build --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke }

Write-Host "[6/7] Run native YOLO smoke"
Invoke-Step {
  .\src-tauri\target\debug\scanner-native-yolo-smoke.exe $SmokeImagePath --raw-rgba --max-width 320 --max-height 180
}

Write-Host "[7/7] Git diff whitespace check"
Invoke-Step { git diff --check }
