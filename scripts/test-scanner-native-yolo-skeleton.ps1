param(
  [switch]$SkipTypeScript,
  [string]$SmokeImagePath
)

$ErrorActionPreference = "Stop"

if (-not $SkipTypeScript) {
  Write-Host "[1/4] TypeScript typecheck"
  pnpm exec tsc --noEmit --pretty false
}

Write-Host "[2/4] Rust cargo check"
cargo check --manifest-path src-tauri/Cargo.toml

Write-Host "[3/4] Native runtime probe"
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe

if ($SmokeImagePath) {
  Write-Host "[4/4] Native smoke inference"
  cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke -- $SmokeImagePath
}
