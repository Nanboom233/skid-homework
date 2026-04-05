const FLATTEN_DETECTION_BAND_RATIO = 0.22;
const FLATTEN_APPLY_BAND_RATIO = 0.22;
const FLATTEN_MIN_BAND_PX = 24;
const FLATTEN_MAX_BAND_RATIO = 0.45;
const FLATTEN_MIN_VALID_ROWS_RATIO = 0.18;
const FLATTEN_MIN_MEAN_SHIFT_PX = 3.5;
const FLATTEN_MIN_MAX_SHIFT_PX = 7;
const FLATTEN_MAX_SHIFT_PX = 14;
const FLATTEN_SMOOTHING_RADIUS = 4;
const FLATTEN_EDGE_ANCHOR_PX = 12;
const FLATTEN_MIN_EDGE_ANCHOR_RATIO = 0.2;
const FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO = 1.35;
const FLATTEN_DOMINANT_SCORE_RATIO = 1.15;

type FlattenSide = "left" | "right";

interface FlattenCandidate {
  side: FlattenSide;
  rowShifts: number[];
  score: number;
  maxShift: number;
  edgeAnchorRatio: number;
}

export interface LocalFlatteningResult {
  imageData: ImageData;
  applied: boolean;
}

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const toGrayscale = (imageData: ImageData): Float32Array => {
  const output = new Float32Array(imageData.width * imageData.height);
  for (let index = 0; index < output.length; index += 1) {
    const offset = index * 4;
    output[index] = (imageData.data[offset] * 0.299)
      + (imageData.data[offset + 1] * 0.587)
      + (imageData.data[offset + 2] * 0.114);
  }

  return output;
};

const computeDarknessWindow = (
  gray: Float32Array,
  width: number,
  height: number,
  x: number,
  y: number,
): number => {
  let darkness = 0;
  let samples = 0;

  for (let sampleY = Math.max(0, y - 1); sampleY <= Math.min(height - 1, y + 1); sampleY += 1) {
    for (let sampleX = Math.max(0, x - 1); sampleX <= Math.min(width - 1, x + 1); sampleX += 1) {
      darkness += 255 - gray[(sampleY * width) + sampleX];
      samples += 1;
    }
  }

  return samples === 0 ? 0 : darkness / samples;
};

const detectEdgeOffsetForRow = (
  gray: Float32Array,
  width: number,
  height: number,
  row: number,
  side: FlattenSide,
  bandWidth: number,
): number | null => {
  let rowDarkness = 0;
  for (let x = 0; x < width; x += 1) {
    rowDarkness += 255 - gray[(row * width) + x];
  }
  const threshold = Math.max(24, (rowDarkness / width) * 2.4);

  if (side === "left") {
    for (let x = 0; x < bandWidth; x += 1) {
      if (computeDarknessWindow(gray, width, height, x, row) >= threshold) {
        return x;
      }
    }
    return null;
  }

  for (let offset = 0; offset < bandWidth; offset += 1) {
    const x = width - 1 - offset;
    if (computeDarknessWindow(gray, width, height, x, row) >= threshold) {
      return offset;
    }
  }

  return null;
};

const percentile = (values: number[], ratio: number): number => {
  if (values.length === 0) {
    return 0;
  }

  const sorted = [...values].sort((first, second) => first - second);
  const index = clamp(Math.floor((sorted.length - 1) * ratio), 0, sorted.length - 1);
  return sorted[index];
};

const fillMissingOffsets = (values: Array<number | null>, fallback: number): number[] => {
  const result = values.map((value) => value ?? Number.NaN);
  let lastKnownIndex = -1;

  for (let index = 0; index < result.length; index += 1) {
    if (!Number.isNaN(result[index])) {
      if (lastKnownIndex < 0) {
        for (let fillIndex = 0; fillIndex < index; fillIndex += 1) {
          result[fillIndex] = result[index];
        }
      } else if (index - lastKnownIndex > 1) {
        const startValue = result[lastKnownIndex];
        const endValue = result[index];
        const gap = index - lastKnownIndex;
        for (let fillIndex = 1; fillIndex < gap; fillIndex += 1) {
          const progress = fillIndex / gap;
          result[lastKnownIndex + fillIndex] = startValue + ((endValue - startValue) * progress);
        }
      }
      lastKnownIndex = index;
    }
  }

  if (lastKnownIndex >= 0) {
    for (let index = lastKnownIndex + 1; index < result.length; index += 1) {
      result[index] = result[lastKnownIndex];
    }
  }

  return result.map((value) => (Number.isNaN(value) ? fallback : value));
};

const smoothValues = (values: number[], radius: number): number[] => {
  return values.map((_, index) => {
    let sum = 0;
    let count = 0;
    for (
      let sampleIndex = Math.max(0, index - radius);
      sampleIndex <= Math.min(values.length - 1, index + radius);
      sampleIndex += 1
    ) {
      sum += values[sampleIndex];
      count += 1;
    }

    return count === 0 ? 0 : sum / count;
  });
};

const buildFlattenCandidate = (
  gray: Float32Array,
  width: number,
  height: number,
  side: FlattenSide,
  bandWidth: number,
): FlattenCandidate | null => {
  const offsets = Array.from({length: height}, (_, row) => {
    return detectEdgeOffsetForRow(gray, width, height, row, side, bandWidth);
  });
  const validOffsets = offsets.filter((value): value is number => value !== null);
  if (validOffsets.length < Math.max(6, Math.floor(height * FLATTEN_MIN_VALID_ROWS_RATIO))) {
    return null;
  }

  const baseline = percentile(validOffsets, 0.15);
  const edgeAnchorRatio = validOffsets.filter((value) => value <= FLATTEN_EDGE_ANCHOR_PX).length
    / validOffsets.length;
  if (edgeAnchorRatio < FLATTEN_MIN_EDGE_ANCHOR_RATIO) {
    return null;
  }
  const rawShifts = offsets.map((value) => {
    return value === null
      ? null
      : clamp(value - baseline, 0, FLATTEN_MAX_SHIFT_PX);
  });
  const filledShifts = fillMissingOffsets(rawShifts, 0);
  const smoothedShifts = smoothValues(filledShifts, FLATTEN_SMOOTHING_RADIUS);
  const positiveShifts = smoothedShifts.filter((value) => value > 0.5);
  if (positiveShifts.length === 0) {
    return null;
  }

  const meanShift = positiveShifts.reduce((sum, value) => sum + value, 0) / positiveShifts.length;
  const maxShift = Math.max(...positiveShifts);
  if (meanShift < FLATTEN_MIN_MEAN_SHIFT_PX || maxShift < FLATTEN_MIN_MAX_SHIFT_PX) {
    return null;
  }

  const coverage = positiveShifts.length / smoothedShifts.length;
  return {
    side,
    rowShifts: smoothedShifts,
    maxShift,
    score: meanShift * (1 + coverage),
    edgeAnchorRatio,
  };
};

const selectFlattenCandidate = (candidates: FlattenCandidate[]): FlattenCandidate | null => {
  if (candidates.length === 0) {
    return null;
  }

  const sorted = [...candidates].sort((first, second) => {
    if (second.edgeAnchorRatio !== first.edgeAnchorRatio) {
      return second.edgeAnchorRatio - first.edgeAnchorRatio;
    }
    if (second.score !== first.score) {
      return second.score - first.score;
    }
    return second.maxShift - first.maxShift;
  });

  const selected = sorted[0];
  if (selected.maxShift < FLATTEN_MIN_MAX_SHIFT_PX) {
    return null;
  }

  const runnerUp = sorted[1];
  if (runnerUp) {
    const edgeAnchorDominant = selected.edgeAnchorRatio
      >= (runnerUp.edgeAnchorRatio * FLATTEN_DOMINANT_EDGE_ANCHOR_RATIO);
    const scoreDominant = selected.score >= (runnerUp.score * FLATTEN_DOMINANT_SCORE_RATIO);
    if (!edgeAnchorDominant && !scoreDominant) {
      return null;
    }
  }

  return selected;
};

const sampleRgbaBilinear = (
  data: Uint8ClampedArray,
  width: number,
  height: number,
  x: number,
  y: number,
): [number, number, number, number] => {
  const clampedX = clamp(x, 0, Math.max(0, width - 1));
  const clampedY = clamp(y, 0, Math.max(0, height - 1));
  const x0 = Math.floor(clampedX);
  const y0 = Math.floor(clampedY);
  const x1 = Math.min(width - 1, x0 + 1);
  const y1 = Math.min(height - 1, y0 + 1);
  const tx = clampedX - x0;
  const ty = clampedY - y0;

  const readPixel = (pixelX: number, pixelY: number): [number, number, number, number] => {
    const offset = ((pixelY * width) + pixelX) * 4;
    return [
      data[offset],
      data[offset + 1],
      data[offset + 2],
      data[offset + 3],
    ];
  };

  const topLeft = readPixel(x0, y0);
  const topRight = readPixel(x1, y0);
  const bottomLeft = readPixel(x0, y1);
  const bottomRight = readPixel(x1, y1);
  const output: [number, number, number, number] = [0, 0, 0, 0];

  for (let channel = 0; channel < 4; channel += 1) {
    const top = topLeft[channel] + ((topRight[channel] - topLeft[channel]) * tx);
    const bottom = bottomLeft[channel] + ((bottomRight[channel] - bottomLeft[channel]) * tx);
    output[channel] = top + ((bottom - top) * ty);
  }

  return output;
};

const applyWarp = (
  imageData: ImageData,
  side: FlattenSide,
  rowShifts: number[],
): ImageData => {
  const applyBandWidth = clamp(
    Math.round(imageData.width * FLATTEN_APPLY_BAND_RATIO),
    Math.max(FLATTEN_MIN_BAND_PX, Math.floor(imageData.width * 0.12)),
    Math.max(FLATTEN_MIN_BAND_PX, Math.floor(imageData.width * FLATTEN_MAX_BAND_RATIO)),
  );
  const output = new Uint8ClampedArray(imageData.data.length);

  for (let y = 0; y < imageData.height; y += 1) {
    const shift = rowShifts[y] ?? 0;
    for (let x = 0; x < imageData.width; x += 1) {
      const distanceIntoBand = side === "left"
        ? x
        : (imageData.width - 1) - x;
      const weight = clamp(1 - (distanceIntoBand / applyBandWidth), 0, 1);
      const easedWeight = weight * weight;
      const sourceX = side === "left"
        ? x + (shift * easedWeight)
        : x - (shift * easedWeight);
      const [red, green, blue, alpha] = sampleRgbaBilinear(
        imageData.data,
        imageData.width,
        imageData.height,
        sourceX,
        y,
      );
      const offset = ((y * imageData.width) + x) * 4;
      output[offset] = red;
      output[offset + 1] = green;
      output[offset + 2] = blue;
      output[offset + 3] = alpha;
    }
  }

  return new ImageData(output, imageData.width, imageData.height);
};

export const applyLocalSpineFlatteningToImageData = (
  imageData: ImageData,
): LocalFlatteningResult => {
  if (imageData.width < 48 || imageData.height < 48) {
    return {
      imageData,
      applied: false,
    };
  }

  const gray = toGrayscale(imageData);
  const bandWidth = clamp(
    Math.round(imageData.width * FLATTEN_DETECTION_BAND_RATIO),
    FLATTEN_MIN_BAND_PX,
    Math.max(FLATTEN_MIN_BAND_PX, Math.floor(imageData.width * 0.33)),
  );
  const candidates = [
    buildFlattenCandidate(gray, imageData.width, imageData.height, "left", bandWidth),
    buildFlattenCandidate(gray, imageData.width, imageData.height, "right", bandWidth),
  ].filter((candidate): candidate is FlattenCandidate => candidate !== null);
  const selected = selectFlattenCandidate(candidates);
  if (!selected) {
    return {
      imageData,
      applied: false,
    };
  }

  return {
    imageData: applyWarp(imageData, selected.side, selected.rowShifts),
    applied: true,
  };
};
