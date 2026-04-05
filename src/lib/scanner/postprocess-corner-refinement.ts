import type {Point} from "./document-detector";

const SAMPLE_MARGIN_RATIO = 0.12;
const MIN_SAMPLE_COUNT = 8;
const MAX_SAMPLE_COUNT = 32;
const SAMPLE_SPACING_PX = 18;
const MIN_SEARCH_RADIUS_PX = 4;
const MAX_SEARCH_RADIUS_PX = 28;
const SEARCH_RADIUS_RATIO = 0.08;
const MAX_CORNER_SHIFT_RATIO = 0.10;
const MIN_CORNER_SHIFT_PX = 8;
const MAX_CORNER_SHIFT_PX = 20;
const MIN_APPLIED_DELTA_PX = 0.75;

interface LineFit {
  point: Point;
  direction: Point;
}

export interface RefinedDocumentQuadResult {
  points: Point[];
  applied: boolean;
}

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const distanceBetweenPoints = (first: Point, second: Point): number => {
  return Math.hypot(first.x - second.x, first.y - second.y);
};

const computePolygonArea = (points: Point[]): number => {
  if (points.length < 3) {
    return 0;
  }

  let area = 0;
  for (let index = 0; index < points.length; index += 1) {
    const current = points[index];
    const next = points[(index + 1) % points.length];
    area += (current.x * next.y) - (next.x * current.y);
  }

  return Math.abs(area) / 2;
};

const orderPoints = (points: Point[]): Point[] => {
  if (points.length !== 4) {
    return points;
  }

  const sums = points.map((point) => point.x + point.y);
  const diffs = points.map((point) => point.y - point.x);

  return [
    points[sums.indexOf(Math.min(...sums))],
    points[diffs.indexOf(Math.min(...diffs))],
    points[sums.indexOf(Math.max(...sums))],
    points[diffs.indexOf(Math.max(...diffs))],
  ];
};

const toGrayscale = (imageData: ImageData): Float32Array => {
  const {data, width, height} = imageData;
  const output = new Float32Array(width * height);

  for (let pixelIndex = 0; pixelIndex < width * height; pixelIndex += 1) {
    const offset = pixelIndex * 4;
    output[pixelIndex] = (data[offset] * 0.299)
      + (data[offset + 1] * 0.587)
      + (data[offset + 2] * 0.114);
  }

  return output;
};

const sampleGray = (
  gray: Float32Array,
  width: number,
  height: number,
  x: number,
  y: number,
): number | null => {
  if (
    x < 0
    || y < 0
    || x > width - 1
    || y > height - 1
  ) {
    return null;
  }

  const x0 = Math.floor(x);
  const y0 = Math.floor(y);
  const x1 = Math.min(width - 1, x0 + 1);
  const y1 = Math.min(height - 1, y0 + 1);
  const tx = x - x0;
  const ty = y - y0;

  const topLeft = gray[(y0 * width) + x0];
  const topRight = gray[(y0 * width) + x1];
  const bottomLeft = gray[(y1 * width) + x0];
  const bottomRight = gray[(y1 * width) + x1];

  const top = topLeft + ((topRight - topLeft) * tx);
  const bottom = bottomLeft + ((bottomRight - bottomLeft) * tx);
  return top + ((bottom - top) * ty);
};

const computeNormalEdgeStrength = (
  gray: Float32Array,
  width: number,
  height: number,
  centerX: number,
  centerY: number,
  normalX: number,
  normalY: number,
  offset: number,
): number | null => {
  const outsideNear = sampleGray(
    gray,
    width,
    height,
    centerX + ((offset - 1) * normalX),
    centerY + ((offset - 1) * normalY),
  );
  const outsideFar = sampleGray(
    gray,
    width,
    height,
    centerX + ((offset - 2) * normalX),
    centerY + ((offset - 2) * normalY),
  );
  const insideNear = sampleGray(
    gray,
    width,
    height,
    centerX + ((offset + 1) * normalX),
    centerY + ((offset + 1) * normalY),
  );
  const insideFar = sampleGray(
    gray,
    width,
    height,
    centerX + ((offset + 2) * normalX),
    centerY + ((offset + 2) * normalY),
  );

  if (
    outsideNear === null
    || outsideFar === null
    || insideNear === null
    || insideFar === null
  ) {
    return null;
  }

  const outside = (outsideNear + outsideFar) * 0.5;
  const inside = (insideNear + insideFar) * 0.5;
  return Math.max(0, inside - outside);
};

const fitLineToPoints = (points: Point[]): LineFit | null => {
  if (points.length < 2) {
    return null;
  }

  const centroid = points.reduce<Point>(
    (accumulator, point) => ({
      x: accumulator.x + point.x,
      y: accumulator.y + point.y,
    }),
    {x: 0, y: 0},
  );
  centroid.x /= points.length;
  centroid.y /= points.length;

  let covarianceXx = 0;
  let covarianceXy = 0;
  let covarianceYy = 0;
  for (const point of points) {
    const deltaX = point.x - centroid.x;
    const deltaY = point.y - centroid.y;
    covarianceXx += deltaX * deltaX;
    covarianceXy += deltaX * deltaY;
    covarianceYy += deltaY * deltaY;
  }

  const angle = 0.5 * Math.atan2(2 * covarianceXy, covarianceXx - covarianceYy);
  const direction = {
    x: Math.cos(angle),
    y: Math.sin(angle),
  };

  if (!Number.isFinite(direction.x) || !Number.isFinite(direction.y)) {
    return null;
  }

  return {
    point: centroid,
    direction,
  };
};

const intersectLines = (first: LineFit, second: LineFit): Point | null => {
  const denominator = (first.direction.x * second.direction.y)
    - (first.direction.y * second.direction.x);
  if (Math.abs(denominator) < 1e-3) {
    return null;
  }

  const deltaX = second.point.x - first.point.x;
  const deltaY = second.point.y - first.point.y;
  const distanceAlongFirst = ((deltaX * second.direction.y) - (deltaY * second.direction.x))
    / denominator;

  return {
    x: first.point.x + (distanceAlongFirst * first.direction.x),
    y: first.point.y + (distanceAlongFirst * first.direction.y),
  };
};

const clampPointToImageBounds = (
  point: Point,
  width: number,
  height: number,
): Point => {
  return {
    x: clamp(point.x, 0, Math.max(0, width - 1)),
    y: clamp(point.y, 0, Math.max(0, height - 1)),
  };
};

const limitPointShift = (candidate: Point, fallback: Point, maxShift: number): Point => {
  const deltaX = candidate.x - fallback.x;
  const deltaY = candidate.y - fallback.y;
  const distance = Math.hypot(deltaX, deltaY);
  if (!Number.isFinite(distance) || distance <= maxShift) {
    return candidate;
  }

  const scale = maxShift / distance;
  return {
    x: fallback.x + (deltaX * scale),
    y: fallback.y + (deltaY * scale),
  };
};

const collectRefinedEdgeSamples = (
  gray: Float32Array,
  width: number,
  height: number,
  start: Point,
  end: Point,
  searchRadius: number,
): Point[] => {
  const edgeLength = distanceBetweenPoints(start, end);
  if (!Number.isFinite(edgeLength) || edgeLength < 1) {
    return [];
  }

  const tangentX = (end.x - start.x) / edgeLength;
  const tangentY = (end.y - start.y) / edgeLength;
  const normalX = -tangentY;
  const normalY = tangentX;
  const sampleCount = clamp(
    Math.round(edgeLength / SAMPLE_SPACING_PX),
    MIN_SAMPLE_COUNT,
    MAX_SAMPLE_COUNT,
  );
  const samples: Array<{point: Point; strength: number}> = [];

  for (let index = 0; index < sampleCount; index += 1) {
    const progress = sampleCount === 1
      ? 0.5
      : SAMPLE_MARGIN_RATIO
        + (((1 - (SAMPLE_MARGIN_RATIO * 2)) * index) / (sampleCount - 1));
    const centerX = start.x + ((end.x - start.x) * progress);
    const centerY = start.y + ((end.y - start.y) * progress);

    let bestStrength = Number.NEGATIVE_INFINITY;
    let bestPoint: Point | null = null;

    for (let offset = -searchRadius; offset <= searchRadius; offset += 1) {
      const strength = computeNormalEdgeStrength(
        gray,
        width,
        height,
        centerX,
        centerY,
        normalX,
        normalY,
        offset,
      );
      if (strength === null || strength <= bestStrength) {
        continue;
      }

      bestStrength = strength;
      bestPoint = {
        x: centerX + (offset * normalX),
        y: centerY + (offset * normalY),
      };
    }

    if (bestPoint && Number.isFinite(bestStrength)) {
      samples.push({
        point: bestPoint,
        strength: bestStrength,
      });
    }
  }

  if (samples.length < 2) {
    return [];
  }

  const averageStrength = samples.reduce((sum, sample) => sum + sample.strength, 0) / samples.length;
  const threshold = averageStrength * 0.6;
  const filtered = samples
    .filter((sample) => sample.strength >= threshold)
    .map((sample) => sample.point);

  return filtered.length >= 2 ? filtered : samples.map((sample) => sample.point);
};

export const refineDocumentQuadInImageData = (
  imageData: ImageData,
  coarsePoints: Point[] | null,
): RefinedDocumentQuadResult => {
  if (!coarsePoints || coarsePoints.length !== 4) {
    return {
      points: coarsePoints ?? [],
      applied: false,
    };
  }

  const orderedCoarse = orderPoints(coarsePoints.map((point) => ({...point})));
  const sideLengths = [
    distanceBetweenPoints(orderedCoarse[0], orderedCoarse[1]),
    distanceBetweenPoints(orderedCoarse[1], orderedCoarse[2]),
    distanceBetweenPoints(orderedCoarse[2], orderedCoarse[3]),
    distanceBetweenPoints(orderedCoarse[3], orderedCoarse[0]),
  ].filter((value) => Number.isFinite(value) && value > 0);

  if (sideLengths.length === 0) {
    return {
      points: orderedCoarse,
      applied: false,
    };
  }

  const shortestSide = Math.min(...sideLengths);
  const searchRadius = clamp(
    Math.round(shortestSide * SEARCH_RADIUS_RATIO),
    MIN_SEARCH_RADIUS_PX,
    MAX_SEARCH_RADIUS_PX,
  );
  const maxCornerShift = clamp(
    shortestSide * MAX_CORNER_SHIFT_RATIO,
    MIN_CORNER_SHIFT_PX,
    MAX_CORNER_SHIFT_PX,
  );
  const gray = toGrayscale(imageData);
  const edges: Array<[Point, Point]> = [
    [orderedCoarse[0], orderedCoarse[1]],
    [orderedCoarse[1], orderedCoarse[2]],
    [orderedCoarse[2], orderedCoarse[3]],
    [orderedCoarse[3], orderedCoarse[0]],
  ];
  const fittedLines = edges.map(([start, end]) => {
    const samples = collectRefinedEdgeSamples(
      gray,
      imageData.width,
      imageData.height,
      start,
      end,
      searchRadius,
    );
    return fitLineToPoints(samples);
  });

  const refinedCandidates = [
    fittedLines[0] && fittedLines[3] ? intersectLines(fittedLines[0], fittedLines[3]) : null,
    fittedLines[0] && fittedLines[1] ? intersectLines(fittedLines[0], fittedLines[1]) : null,
    fittedLines[1] && fittedLines[2] ? intersectLines(fittedLines[1], fittedLines[2]) : null,
    fittedLines[2] && fittedLines[3] ? intersectLines(fittedLines[2], fittedLines[3]) : null,
  ];

  const centroid: Point = {
    x: orderedCoarse.reduce((sum, p) => sum + p.x, 0) / 4,
    y: orderedCoarse.reduce((sum, p) => sum + p.y, 0) / 4,
  };
  const imageBoundaryMargin = shortestSide * 0.04;

  const refinedPoints = orderedCoarse.map((fallbackPoint, index) => {
    const candidate = refinedCandidates[index];
    if (!candidate) {
      return fallbackPoint;
    }

    const nearBoundary =
      fallbackPoint.x < imageBoundaryMargin
      || fallbackPoint.y < imageBoundaryMargin
      || fallbackPoint.x > imageData.width - imageBoundaryMargin
      || fallbackPoint.y > imageData.height - imageBoundaryMargin;

    if (nearBoundary) {
      const coarseToCenter = Math.hypot(centroid.x - fallbackPoint.x, centroid.y - fallbackPoint.y);
      const candidateToCenter = Math.hypot(centroid.x - candidate.x, centroid.y - candidate.y);
      if (candidateToCenter < coarseToCenter) {
        // Corner near image edge + refinement pulls inward = real page boundary (spine).
        // Skip refinement entirely.
        return fallbackPoint;
      }
    }

    return clampPointToImageBounds(
      limitPointShift(candidate, fallbackPoint, maxCornerShift),
      imageData.width,
      imageData.height,
    );
  });

  const coarseArea = computePolygonArea(orderedCoarse);
  const refinedArea = computePolygonArea(refinedPoints);
  if (
    !Number.isFinite(refinedArea)
    || refinedArea <= 0
    || refinedArea < coarseArea * 0.5
    || refinedArea > coarseArea * 1.5
  ) {
    return {
      points: orderedCoarse,
      applied: false,
    };
  }

  const applied = refinedPoints.some((point, index) => {
    return distanceBetweenPoints(point, orderedCoarse[index]) >= MIN_APPLIED_DELTA_PX;
  });

  return {
    points: refinedPoints,
    applied,
  };
};
