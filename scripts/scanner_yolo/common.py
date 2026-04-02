from __future__ import annotations

from pathlib import Path
from typing import Iterable, Sequence


IMAGE_SUFFIXES = (".png", ".jpg", ".jpeg", ".bmp", ".webp")


def clamp01(value: float) -> float:
    return max(0.0, min(1.0, value))


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def order_quad(points: Sequence[Sequence[float]]) -> list[tuple[float, float]]:
    if len(points) != 4:
        raise ValueError(f"expected 4 points, got {len(points)}")

    quad = [(float(point[0]), float(point[1])) for point in points]
    sums = [x + y for x, y in quad]
    diffs = [y - x for x, y in quad]

    ordered = [
        quad[sums.index(min(sums))],
        quad[diffs.index(min(diffs))],
        quad[sums.index(max(sums))],
        quad[diffs.index(max(diffs))],
    ]

    if len(set(ordered)) == 4:
        return ordered

    by_y = sorted(quad, key=lambda point: (point[1], point[0]))
    top = sorted(by_y[:2], key=lambda point: point[0])
    bottom = sorted(by_y[2:], key=lambda point: point[0])
    return [top[0], top[1], bottom[1], bottom[0]]


def build_output_stem(source_dir: Path, file_path: Path) -> str:
    relative = file_path.relative_to(source_dir).with_suffix("")
    return "__".join(relative.parts)


def find_image_for_json(json_path: Path) -> Path | None:
    for suffix in IMAGE_SUFFIXES:
        candidate = json_path.with_suffix(suffix)
        if candidate.exists():
            return candidate
    return None


def normalize_points(
    points: Sequence[Sequence[float]],
    width: int,
    height: int,
) -> list[tuple[float, float]]:
    if width <= 0 or height <= 0:
        raise ValueError("image width and height must be positive")

    return [
        (
            clamp01(float(x) / float(width)),
            clamp01(float(y) / float(height)),
        )
        for x, y in points
    ]


def bbox_from_points(
    normalized_points: Sequence[Sequence[float]],
) -> tuple[float, float, float, float]:
    xs = [float(point[0]) for point in normalized_points]
    ys = [float(point[1]) for point in normalized_points]
    min_x = clamp01(min(xs))
    min_y = clamp01(min(ys))
    max_x = clamp01(max(xs))
    max_y = clamp01(max(ys))
    return (
        clamp01((min_x + max_x) / 2.0),
        clamp01((min_y + max_y) / 2.0),
        clamp01(max_x - min_x),
        clamp01(max_y - min_y),
    )


def build_yolo_pose_label(
    points: Sequence[Sequence[float]],
    width: int,
    height: int,
    class_id: int = 0,
    visibility: int = 2,
) -> str:
    ordered_points = order_quad(points)
    normalized_points = normalize_points(ordered_points, width, height)
    center_x, center_y, box_width, box_height = bbox_from_points(normalized_points)

    values = [
        str(class_id),
        f"{center_x:.6f}",
        f"{center_y:.6f}",
        f"{box_width:.6f}",
        f"{box_height:.6f}",
    ]
    for x, y in normalized_points:
        values.extend((f"{x:.6f}", f"{y:.6f}", str(visibility)))
    return " ".join(values)


def write_dataset_yaml(
    output_dir: Path,
    dataset_name: str = "document-corners-pose",
) -> Path:
    yaml_path = output_dir / "dataset.yaml"
    yaml_path.write_text(
        "\n".join(
            [
                f"# {dataset_name}",
                "path: .",
                "train: images/train",
                "val: images/val",
                "test: images/test",
                "kpt_shape: [4, 3]",
                "flip_idx: [1, 0, 3, 2]",
                "names:",
                "  0: document",
                "",
            ]
        ),
        encoding="utf-8",
    )
    return yaml_path


def write_lines(path: Path, lines: Iterable[str]) -> None:
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
