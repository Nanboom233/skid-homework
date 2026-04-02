param(
  [string]$WorkspaceRoot = ".tmp/scanner-yolo-data-pipeline"
)

$ErrorActionPreference = "Stop"

$workspace = Resolve-Path "." | ForEach-Object { Join-Path $_ $WorkspaceRoot }
if (Test-Path $workspace) {
  Remove-Item -LiteralPath $workspace -Recurse -Force
}

New-Item -ItemType Directory -Path $workspace | Out-Null

$fixturesDir = Join-Path $workspace "fixtures"
$pageDir = Join-Path $fixturesDir "pages"
$backgroundDir = Join-Path $fixturesDir "backgrounds"
$midvDir = Join-Path $fixturesDir "midv"
$midvOut = Join-Path $workspace "midv-out"
$syntheticOut = Join-Path $workspace "synthetic-out"
$syntheticProceduralOut = Join-Path $workspace "synthetic-out-procedural"

New-Item -ItemType Directory -Path $pageDir, $backgroundDir, $midvDir | Out-Null

Write-Host "[1/5] Compile Python scripts"
python -m compileall scripts/scanner_yolo

Write-Host "[2/5] Create smoke fixtures"
@'
from pathlib import Path
import json
from PIL import Image, ImageDraw

workspace = Path(r"__WORKSPACE__")
page_dir = workspace / "fixtures" / "pages"
background_dir = workspace / "fixtures" / "backgrounds"
midv_dir = workspace / "fixtures" / "midv"

page_dir.mkdir(parents=True, exist_ok=True)
background_dir.mkdir(parents=True, exist_ok=True)
midv_dir.mkdir(parents=True, exist_ok=True)

for index in range(2):
    image = Image.new("RGB", (900, 1200), (247, 245, 240))
    draw = ImageDraw.Draw(image)
    draw.rectangle((60, 60, 840, 1140), outline=(28, 28, 28), width=8)
    draw.text((120, 140), f"Page {index + 1}", fill=(20, 20, 20))
    draw.rectangle((120, 220, 780, 320), fill=(230, 235, 255))
    draw.rectangle((120, 360, 780, 420), fill=(235, 255, 235))
    image.save(page_dir / f"page_{index + 1}.png")

for index in range(2):
    image = Image.new("RGB", (1400, 1000), (160 + index * 18, 150, 135 + index * 12))
    draw = ImageDraw.Draw(image)
    for stripe in range(0, 1400, 48):
        draw.rectangle((stripe, 0, stripe + 18, 1000), fill=(110 + index * 10, 100, 88))
    image.save(background_dir / f"bg_{index + 1}.png")

frame = Image.new("RGB", (640, 480), (92, 110, 126))
draw = ImageDraw.Draw(frame)
quad = [(128, 72), (520, 96), (548, 390), (110, 412)]
draw.polygon(quad, fill=(245, 244, 240), outline=(12, 12, 12))
frame.save(midv_dir / "frame_0001.png")
(midv_dir / "frame_0001.json").write_text(json.dumps({"quad": quad}), encoding="utf-8")
'@.Replace("__WORKSPACE__", $workspace) | python -

Write-Host "[3/5] Run MIDV conversion smoke"
python scripts/scanner_yolo/convert_midv_to_yolo_pose.py `
  --source-dir $midvDir `
  --output-dir $midvOut `
  --split train `
  --write-dataset-yaml

Write-Host "[4/5] Run synthetic generation smoke and validate outputs"
python scripts/scanner_yolo/generate_synthetic_pose_dataset.py `
  --page-dir $pageDir `
  --background-dir $backgroundDir `
  --output-dir $syntheticOut `
  --split train `
  --samples 4 `
  --seed 7 `
  --write-dataset-yaml

Write-Host "[5/5] Run synthetic generation smoke without backgrounds"
python scripts/scanner_yolo/generate_synthetic_pose_dataset.py `
  --page-dir $pageDir `
  --output-dir $syntheticProceduralOut `
  --split val `
  --samples 2 `
  --seed 11

@'
from pathlib import Path

workspace = Path(r"__WORKSPACE__")
midv_out = workspace / "midv-out"
synthetic_out = workspace / "synthetic-out"
synthetic_procedural_out = workspace / "synthetic-out-procedural"

midv_label = next((midv_out / "labels" / "train").glob("*.txt"))
midv_tokens = midv_label.read_text(encoding="utf-8").strip().split()
assert len(midv_tokens) == 17, f"expected 17 label tokens, got {len(midv_tokens)}"
assert (midv_out / "dataset.yaml").exists(), "MIDV dataset.yaml missing"

synthetic_images = sorted((synthetic_out / "images" / "train").glob("*.png"))
synthetic_labels = sorted((synthetic_out / "labels" / "train").glob("*.txt"))
assert len(synthetic_images) == 4, f"expected 4 synthetic images, got {len(synthetic_images)}"
assert len(synthetic_labels) == 4, f"expected 4 synthetic labels, got {len(synthetic_labels)}"
for label_path in synthetic_labels:
    tokens = label_path.read_text(encoding="utf-8").strip().split()
    assert len(tokens) == 17, f"{label_path} should contain 17 label tokens"
assert (synthetic_out / "dataset.yaml").exists(), "synthetic dataset.yaml missing"

procedural_images = sorted((synthetic_procedural_out / "images" / "val").glob("*.png"))
procedural_labels = sorted((synthetic_procedural_out / "labels" / "val").glob("*.txt"))
assert len(procedural_images) == 2, f"expected 2 procedural synthetic images, got {len(procedural_images)}"
assert len(procedural_labels) == 2, f"expected 2 procedural synthetic labels, got {len(procedural_labels)}"
for label_path in procedural_labels:
    tokens = label_path.read_text(encoding="utf-8").strip().split()
    assert len(tokens) == 17, f"{label_path} should contain 17 label tokens"

print("scanner yolo data pipeline smoke test passed")
'@.Replace("__WORKSPACE__", $workspace) | python -
