from __future__ import annotations

import argparse
import json
import math
import random
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageEnhance, ImageFilter

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from common import IMAGE_SUFFIXES, build_yolo_pose_label, ensure_dir, order_quad, write_dataset_yaml


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate a synthetic document-corner dataset for Ultralytics YOLO pose training.",
    )
    parser.add_argument("--page-dir", type=Path, required=True, help="Directory with source page images.")
    parser.add_argument("--background-dir", type=Path, help="Directory with background images. Procedural backgrounds are used if omitted.")
    parser.add_argument("--output-dir", type=Path, required=True, help="Directory to write the synthetic dataset.")
    parser.add_argument("--split", default="train", choices=("train", "val", "test"), help="Dataset split to populate.")
    parser.add_argument("--samples", type=int, default=128, help="Number of synthetic samples to generate.")
    parser.add_argument("--width", type=int, default=1280, help="Output canvas width.")
    parser.add_argument("--height", type=int, default=960, help="Output canvas height.")
    parser.add_argument("--seed", type=int, default=7, help="Random seed for reproducible generation.")
    parser.add_argument("--class-id", type=int, default=0, help="Class id used in the YOLO label.")
    parser.add_argument(
        "--write-dataset-yaml",
        action="store_true",
        help="Write dataset.yaml to the output root after generation.",
    )
    return parser.parse_args()


def list_images(directory: Path | None) -> list[Path]:
    if directory is None or not directory.exists():
        return []
    return sorted(path for path in directory.rglob("*") if path.suffix.lower() in IMAGE_SUFFIXES)


def fit_cover(image: Image.Image, size: tuple[int, int]) -> Image.Image:
    width, height = size
    source_width, source_height = image.size
    scale = max(width / source_width, height / source_height)
    resized = image.resize(
        (int(math.ceil(source_width * scale)), int(math.ceil(source_height * scale))),
        Image.Resampling.BICUBIC,
    )
    left = max(0, (resized.width - width) // 2)
    top = max(0, (resized.height - height) // 2)
    return resized.crop((left, top, left + width, top + height))


def make_procedural_background(size: tuple[int, int], rng: random.Random) -> Image.Image:
    width, height = size
    base = Image.new(
        "RGB",
        size,
        color=(
            rng.randint(150, 220),
            rng.randint(150, 220),
            rng.randint(150, 220),
        ),
    )
    draw = ImageDraw.Draw(base, "RGBA")
    for _ in range(18):
        x1 = rng.randint(0, width)
        y1 = rng.randint(0, height)
        x2 = x1 + rng.randint(width // 8, width // 2)
        y2 = y1 + rng.randint(height // 8, height // 2)
        color = (
            rng.randint(50, 200),
            rng.randint(50, 200),
            rng.randint(50, 200),
            rng.randint(18, 56),
        )
        draw.ellipse((x1, y1, x2, y2), fill=color)
    return base.filter(ImageFilter.GaussianBlur(radius=4))


def pick_background(background_paths: list[Path], canvas_size: tuple[int, int], rng: random.Random) -> Image.Image:
    if not background_paths:
        return make_procedural_background(canvas_size, rng)

    with Image.open(rng.choice(background_paths)) as image:
        return fit_cover(image.convert("RGB"), canvas_size)


def compute_perspective_coefficients(
    destination_points: list[tuple[float, float]],
    source_points: list[tuple[float, float]],
) -> list[float]:
    matrix = []
    vector = []
    for (x_dst, y_dst), (x_src, y_src) in zip(destination_points, source_points):
        matrix.append([x_dst, y_dst, 1, 0, 0, 0, -x_src * x_dst, -x_src * y_dst])
        matrix.append([0, 0, 0, x_dst, y_dst, 1, -y_src * x_dst, -y_src * y_dst])
        vector.append(x_src)
        vector.append(y_src)

    solution = np.linalg.solve(np.asarray(matrix, dtype=float), np.asarray(vector, dtype=float))
    return solution.tolist()


def sample_destination_quad(
    canvas_size: tuple[int, int],
    page_size: tuple[int, int],
    rng: random.Random,
) -> list[tuple[float, float]]:
    canvas_width, canvas_height = canvas_size
    page_width, page_height = page_size
    scale = rng.uniform(0.42, 0.8)
    target_width = canvas_width * scale
    target_height = target_width * (page_height / page_width)
    if target_height > canvas_height * 0.8:
        target_height = canvas_height * 0.8
        target_width = target_height * (page_width / page_height)

    center_x = rng.uniform(target_width * 0.7, canvas_width - target_width * 0.7)
    center_y = rng.uniform(target_height * 0.7, canvas_height - target_height * 0.7)
    jitter_x = target_width * 0.18
    jitter_y = target_height * 0.18

    rectangle = [
        (-target_width / 2.0, -target_height / 2.0),
        (target_width / 2.0, -target_height / 2.0),
        (target_width / 2.0, target_height / 2.0),
        (-target_width / 2.0, target_height / 2.0),
    ]
    quad = []
    for offset_x, offset_y in rectangle:
        quad.append(
            (
                min(max(center_x + offset_x + rng.uniform(-jitter_x, jitter_x), 8.0), canvas_width - 8.0),
                min(max(center_y + offset_y + rng.uniform(-jitter_y, jitter_y), 8.0), canvas_height - 8.0),
            )
        )
    return order_quad(quad)


def build_shadow_layer(
    size: tuple[int, int],
    quad: list[tuple[float, float]],
    rng: random.Random,
) -> Image.Image:
    offset_x = rng.uniform(8.0, 26.0)
    offset_y = rng.uniform(8.0, 24.0)
    shadow_points = [(x + offset_x, y + offset_y) for x, y in quad]
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer, "RGBA")
    draw.polygon(shadow_points, fill=(0, 0, 0, rng.randint(50, 84)))
    return layer.filter(ImageFilter.GaussianBlur(radius=rng.uniform(8.0, 16.0)))


def add_shadow_band(image: Image.Image, rng: random.Random) -> Image.Image:
    overlay = Image.new("RGBA", image.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay, "RGBA")
    width, height = image.size
    band_height = rng.randint(max(24, height // 18), max(48, height // 6))
    y = rng.randint(0, max(1, height - band_height))
    draw.rectangle((0, y, width, y + band_height), fill=(0, 0, 0, rng.randint(12, 42)))
    overlay = overlay.filter(ImageFilter.GaussianBlur(radius=rng.uniform(8.0, 14.0)))
    return Image.alpha_composite(image.convert("RGBA"), overlay)


def add_occlusion(image: Image.Image, quad: list[tuple[float, float]], rng: random.Random) -> Image.Image:
    layer = image.convert("RGBA")
    draw = ImageDraw.Draw(layer, "RGBA")
    xs = [point[0] for point in quad]
    ys = [point[1] for point in quad]
    min_x = int(min(xs))
    max_x = int(max(xs))
    min_y = int(min(ys))
    max_y = int(max(ys))
    occ_width = max(12, (max_x - min_x) // rng.randint(6, 10))
    occ_height = max(12, (max_y - min_y) // rng.randint(6, 10))
    occ_x = rng.randint(min_x, max(min_x, max_x - occ_width))
    occ_y = rng.randint(min_y, max(min_y, max_y - occ_height))
    color = (
        rng.randint(30, 210),
        rng.randint(30, 210),
        rng.randint(30, 210),
        rng.randint(110, 190),
    )
    draw.rounded_rectangle((occ_x, occ_y, occ_x + occ_width, occ_y + occ_height), radius=8, fill=color)
    return layer


def apply_global_augmentations(image: Image.Image, rng: random.Random) -> Image.Image:
    rgb = image.convert("RGB")
    rgb = ImageEnhance.Brightness(rgb).enhance(rng.uniform(0.82, 1.16))
    rgb = ImageEnhance.Contrast(rgb).enhance(rng.uniform(0.86, 1.2))
    rgb = ImageEnhance.Color(rgb).enhance(rng.uniform(0.9, 1.08))
    if rng.random() < 0.35:
        rgb = rgb.filter(ImageFilter.GaussianBlur(radius=rng.uniform(0.4, 1.3)))
    return rgb


def main() -> int:
    args = parse_args()
    rng = random.Random(args.seed)

    page_paths = list_images(args.page_dir.resolve())
    background_paths = list_images(args.background_dir.resolve() if args.background_dir else None)
    if not page_paths:
        raise FileNotFoundError(f"no source page images found under {args.page_dir}")

    output_dir = args.output_dir.resolve()
    image_output_dir = ensure_dir(output_dir / "images" / args.split)
    label_output_dir = ensure_dir(output_dir / "labels" / args.split)
    canvas_size = (args.width, args.height)

    for index in range(args.samples):
        page_path = rng.choice(page_paths)
        background = pick_background(background_paths, canvas_size, rng).convert("RGBA")

        with Image.open(page_path) as page_image:
            page = page_image.convert("RGBA")

        destination_quad = sample_destination_quad(canvas_size, page.size, rng)
        source_quad = [
            (0.0, 0.0),
            (float(page.width - 1), 0.0),
            (float(page.width - 1), float(page.height - 1)),
            (0.0, float(page.height - 1)),
        ]
        coefficients = compute_perspective_coefficients(destination_quad, source_quad)

        shadow = build_shadow_layer(canvas_size, destination_quad, rng)
        warped_page = page.transform(
            canvas_size,
            Image.Transform.PERSPECTIVE,
            coefficients,
            resample=Image.Resampling.BICUBIC,
            fillcolor=(0, 0, 0, 0),
        )

        composed = Image.alpha_composite(background, shadow)
        composed = Image.alpha_composite(composed, warped_page)
        composed = add_shadow_band(composed, rng)
        if rng.random() < 0.3:
            composed = add_occlusion(composed, destination_quad, rng)

        final_image = apply_global_augmentations(composed, rng)
        stem = f"synthetic_{args.split}_{index:06d}"
        image_path = image_output_dir / f"{stem}.png"
        label_path = label_output_dir / f"{stem}.txt"

        final_image.save(image_path)
        label_path.write_text(
            build_yolo_pose_label(destination_quad, args.width, args.height, class_id=args.class_id) + "\n",
            encoding="utf-8",
        )

    if args.write_dataset_yaml:
        write_dataset_yaml(output_dir, dataset_name="synthetic-document-corners-pose")

    print(
        json.dumps(
            {
                "pageDir": str(args.page_dir.resolve()),
                "backgroundDir": str(args.background_dir.resolve()) if args.background_dir else None,
                "outputDir": str(output_dir),
                "split": args.split,
                "samples": args.samples,
                "canvas": {"width": args.width, "height": args.height},
                "seed": args.seed,
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
