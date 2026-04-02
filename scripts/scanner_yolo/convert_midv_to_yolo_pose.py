from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

from PIL import Image

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from common import build_output_stem, build_yolo_pose_label, ensure_dir, find_image_for_json, write_dataset_yaml


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Convert MIDV-style quad annotations into Ultralytics YOLO pose labels with 4 keypoints.",
    )
    parser.add_argument("--source-dir", type=Path, required=True, help="Directory containing MIDV images and JSON quad files.")
    parser.add_argument("--output-dir", type=Path, required=True, help="Directory to write YOLO pose dataset files.")
    parser.add_argument("--split", default="train", choices=("train", "val", "test"), help="Dataset split to populate.")
    parser.add_argument("--class-id", type=int, default=0, help="Class id used in the YOLO label.")
    parser.add_argument(
        "--write-dataset-yaml",
        action="store_true",
        help="Write dataset.yaml to the output root after conversion.",
    )
    return parser.parse_args()


def load_quad(json_path: Path) -> list[list[float]]:
    payload = json.loads(json_path.read_text(encoding="utf-8"))
    quad = payload.get("quad")
    if not isinstance(quad, list) or len(quad) != 4:
        raise ValueError(f"{json_path} does not contain a valid 4-point 'quad' field")
    return quad


def main() -> int:
    args = parse_args()
    source_dir = args.source_dir.resolve()
    output_dir = args.output_dir.resolve()

    if not source_dir.exists():
        raise FileNotFoundError(f"source directory does not exist: {source_dir}")

    image_output_dir = ensure_dir(output_dir / "images" / args.split)
    label_output_dir = ensure_dir(output_dir / "labels" / args.split)

    converted_count = 0
    skipped: list[str] = []

    for json_path in sorted(source_dir.rglob("*.json")):
        image_path = find_image_for_json(json_path)
        if image_path is None:
            skipped.append(f"missing image pair for {json_path}")
            continue

        quad = load_quad(json_path)
        with Image.open(image_path) as image:
            width, height = image.size

        stem = build_output_stem(source_dir, json_path)
        label_line = build_yolo_pose_label(quad, width, height, class_id=args.class_id)

        target_image_path = image_output_dir / f"{stem}{image_path.suffix.lower()}"
        target_label_path = label_output_dir / f"{stem}.txt"

        shutil.copy2(image_path, target_image_path)
        target_label_path.write_text(label_line + "\n", encoding="utf-8")
        converted_count += 1

    if args.write_dataset_yaml:
        write_dataset_yaml(output_dir, dataset_name="midv-document-corners-pose")

    print(
        json.dumps(
            {
                "sourceDir": str(source_dir),
                "outputDir": str(output_dir),
                "split": args.split,
                "converted": converted_count,
                "skipped": skipped,
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
