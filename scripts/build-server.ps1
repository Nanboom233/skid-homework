param(
  [ValidateSet("Debug", "Release")]
  [string]$Variant = "Debug"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$serverDir = Join-Path $repoRoot "server"
$resourceDir = Join-Path $repoRoot "src-tauri/resources"
$resourceJar = Join-Path $resourceDir "camera-server.jar"
$taskName = if ($Variant -eq "Release") { "assembleRelease" } else { "assembleDebug" }
$variantDir = $Variant.ToLowerInvariant()
$builtJar = Join-Path $serverDir "build/outputs/apk/$variantDir/camera-server.jar"

if (Test-Path (Join-Path $serverDir "gradlew.bat")) {
  $gradleCommand = Join-Path $serverDir "gradlew.bat"
  & $gradleCommand "-p" $serverDir $taskName
} elseif (Get-Command gradle -ErrorAction SilentlyContinue) {
  & gradle "-p" $serverDir $taskName
} else {
  throw "Gradle was not found. Install Gradle or add a Gradle wrapper under server/."
}

if (!(Test-Path $builtJar)) {
  throw "Expected camera server artifact was not produced: $builtJar"
}

New-Item -ItemType Directory -Force -Path $resourceDir | Out-Null
Copy-Item -LiteralPath $builtJar -Destination $resourceJar -Force
Write-Host "Copied camera server artifact to $resourceJar"
