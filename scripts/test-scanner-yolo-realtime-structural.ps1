param(
  [string]$SmokeImagePath = ".github/images/skidhw-thumbnail-new.png",
  [int]$Iterations = 5
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

Write-Host "[1/6] TypeScript type check"
Invoke-Step { pnpm exec tsc --noEmit --pretty false }

Write-Host "[2/6] Rust cargo check"
Invoke-Step { cargo check --manifest-path src-tauri/Cargo.toml }

Write-Host "[3/6] Rust cargo test"
Invoke-Step { cargo test --manifest-path src-tauri/Cargo.toml }

Write-Host "[4/6] Build native YOLO benchmark binary"
Invoke-Step { cargo build --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-benchmark }

Write-Host "[5/6] Run native YOLO benchmark"
Invoke-Step {
  .\src-tauri\target\debug\scanner-native-yolo-benchmark.exe $SmokeImagePath --iterations $Iterations
}

Write-Host "[6/6] Run native YOLO smoke"
Invoke-Step {
  .\src-tauri\target\debug\scanner-native-yolo-smoke.exe $SmokeImagePath --raw-rgba --max-width 320 --max-height 180
}
