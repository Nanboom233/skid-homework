import type {Point} from "./types";

export interface DocumentQuadAssessment {
  trustworthy: boolean;
  orderedPoints: Point[] | null;
  areaRatio: number | null;
  widthBalance: number | null;
  heightBalance: number | null;
  diagonalBalance: number | null;
  shortestToLongestEdgeRatio: number | null;
  borderTouchCount: number | null;
  reason: string | null;
}

const MIN_AREA_RATIO = 0.04;
const MIN_CORNER_DISTANCE_RATIO = 0.03;
const MIN_CORNER_DISTANCE_PX = 24;
const MIN_OPPOSITE_EDGE_BALANCE = 0.12;
const MIN_SHORTEST_TO_LONGEST_EDGE_RATIO = 0.08;
const BORDER_TOUCH_MARGIN_RATIO = 0.02;
const BORDER_TOUCH_MARGIN_PX = 4;
const MAX_BORDER_TOUCH_COUNT = 2;

const distanceBetweenPoints = (first: Point, second: Point): number => {
  return Math.hypot(first.x - second.x, first.y - second.y);
};

const computePolygonArea = (points: Point[]): number => {
  if (points.length !== 4) {
    return 0;
  }

  let area = 0;
  for (let index = 0; index < points.length; index += 1) {
    const current = points[index];
    const next = points[(index + 1) % points.length];
    area += (current.x * next.y) - (next.x * current.y);
  }

  return Math.abs(area) * 0.5;
};

export const orderDocumentQuadPoints = (points: Point[] | null | undefined): Point[] | null => {
  if (!points || points.length !== 4) {
    return null;
  }

  if (points.some((point) => !Number.isFinite(point.x) || !Number.isFinite(point.y))) {
    return null;
  }

  const sums = points.map((point) => point.x + point.y);
  const diffs = points.map((point) => point.y - point.x);
  const minSumIndex = sums.reduce((bestIndex, value, index, values) => {
    return value < values[bestIndex] ? index : bestIndex;
  }, 0);
  const minDiffIndex = diffs.reduce((bestIndex, value, index, values) => {
    return value < values[bestIndex] ? index : bestIndex;
  }, 0);
  const maxSumIndex = sums.reduce((bestIndex, value, index, values) => {
    return value > values[bestIndex] ? index : bestIndex;
  }, 0);
  const maxDiffIndex = diffs.reduce((bestIndex, value, index, values) => {
    return value > values[bestIndex] ? index : bestIndex;
  }, 0);

  return [
    points[minSumIndex],
    points[minDiffIndex],
    points[maxSumIndex],
    points[maxDiffIndex],
  ];
};

export const isConvexOrderedQuad = (points: Point[]): boolean => {
  let positive = 0;
  let negative = 0;
  for (let i = 0; i < 4; i += 1) {
    const curr = points[i];
    const next = points[(i + 1) % 4];
    const after = points[(i + 2) % 4];
    const cross = (next.x - curr.x) * (after.y - next.y)
                - (next.y - curr.y) * (after.x - next.x);
    if (cross > 0) positive += 1;
    else if (cross < 0) negative += 1;
  }
  return !(positive > 0 && negative > 0);
};

export const assessDocumentQuad = (
  points: Point[] | null | undefined,
  frameWidth: number,
  frameHeight: number,
): DocumentQuadAssessment => {
  const orderedPoints = orderDocumentQuadPoints(points);
  if (!orderedPoints) {
    return {
      trustworthy: false,
      orderedPoints: null,
      areaRatio: null,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: null,
      borderTouchCount: null,
      reason: "Document quad is missing or malformed.",
    };
  }

  const imageArea = frameWidth * frameHeight;
  if (!Number.isFinite(imageArea) || imageArea <= 0) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio: null,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: null,
      borderTouchCount: null,
      reason: "Frame dimensions are invalid for quad assessment.",
    };
  }

  const area = computePolygonArea(orderedPoints);
  const areaRatio = area / imageArea;
  if (!Number.isFinite(areaRatio) || areaRatio < MIN_AREA_RATIO) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: null,
      borderTouchCount: null,
      reason: "Document quad area is too small for a real page capture.",
    };
  }

  const widthTop = distanceBetweenPoints(orderedPoints[0], orderedPoints[1]);
  const widthBottom = distanceBetweenPoints(orderedPoints[3], orderedPoints[2]);
  const heightLeft = distanceBetweenPoints(orderedPoints[0], orderedPoints[3]);
  const heightRight = distanceBetweenPoints(orderedPoints[1], orderedPoints[2]);
  const diagonalPrimary = distanceBetweenPoints(orderedPoints[0], orderedPoints[2]);
  const diagonalSecondary = distanceBetweenPoints(orderedPoints[1], orderedPoints[3]);
  const edgeLengths = [widthTop, widthBottom, heightLeft, heightRight];
  const shortestEdge = Math.min(...edgeLengths);
  const longestEdge = Math.max(...edgeLengths, 1);
  const minCornerDistance = Math.max(MIN_CORNER_DISTANCE_PX, Math.min(frameWidth, frameHeight) * MIN_CORNER_DISTANCE_RATIO);

  if (!Number.isFinite(shortestEdge) || shortestEdge < minCornerDistance) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: shortestEdge / longestEdge,
      borderTouchCount: null,
      reason: "Document quad collapsed one or more adjacent corners together.",
    };
  }

  const topMidY = (orderedPoints[0].y + orderedPoints[1].y) * 0.5;
  const bottomMidY = (orderedPoints[2].y + orderedPoints[3].y) * 0.5;
  const leftMidX = (orderedPoints[0].x + orderedPoints[3].x) * 0.5;
  const rightMidX = (orderedPoints[1].x + orderedPoints[2].x) * 0.5;
  if (!(topMidY < bottomMidY && leftMidX < rightMidX)) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: shortestEdge / longestEdge,
      borderTouchCount: null,
      reason: "Document quad ordering is inconsistent after normalization.",
    };
  }

  if (!isConvexOrderedQuad(orderedPoints)) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance: null,
      heightBalance: null,
      diagonalBalance: null,
      shortestToLongestEdgeRatio: shortestEdge / longestEdge,
      borderTouchCount: null,
      reason: "Document quad is non-convex or self-intersecting.",
    };
  }

  const widthBalance = Math.min(widthTop, widthBottom) / Math.max(widthTop, widthBottom, 1);
  const heightBalance = Math.min(heightLeft, heightRight) / Math.max(heightLeft, heightRight, 1);
  const diagonalBalance = Math.min(diagonalPrimary, diagonalSecondary) / Math.max(diagonalPrimary, diagonalSecondary, 1);
  const shortestToLongestEdgeRatio = shortestEdge / longestEdge;
  const borderMarginX = Math.max(BORDER_TOUCH_MARGIN_PX, frameWidth * BORDER_TOUCH_MARGIN_RATIO);
  const borderMarginY = Math.max(BORDER_TOUCH_MARGIN_PX, frameHeight * BORDER_TOUCH_MARGIN_RATIO);
  const borderTouchCount = orderedPoints.filter((point) => (
    point.x <= borderMarginX
    || point.x >= frameWidth - borderMarginX
    || point.y <= borderMarginY
    || point.y >= frameHeight - borderMarginY
  )).length;

  if (widthBalance < MIN_OPPOSITE_EDGE_BALANCE || heightBalance < MIN_OPPOSITE_EDGE_BALANCE) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance,
      heightBalance,
      diagonalBalance,
      shortestToLongestEdgeRatio,
      borderTouchCount,
      reason: "Document quad opposite edges are too imbalanced to trust.",
    };
  }

  if (shortestToLongestEdgeRatio < MIN_SHORTEST_TO_LONGEST_EDGE_RATIO) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance,
      heightBalance,
      diagonalBalance,
      shortestToLongestEdgeRatio,
      borderTouchCount,
      reason: "Document quad is too tapered and likely collapsed onto one side.",
    };
  }

  if (
    borderTouchCount > MAX_BORDER_TOUCH_COUNT
    && shortestToLongestEdgeRatio < 0.18
    && (widthBalance < 0.25 || heightBalance < 0.25)
  ) {
    return {
      trustworthy: false,
      orderedPoints,
      areaRatio,
      widthBalance,
      heightBalance,
      diagonalBalance,
      shortestToLongestEdgeRatio,
      borderTouchCount,
      reason: "Document quad touches too many frame borders to trust the mapping.",
    };
  }

  return {
    trustworthy: true,
    orderedPoints,
    areaRatio,
    widthBalance,
    heightBalance,
    diagonalBalance,
    shortestToLongestEdgeRatio,
    borderTouchCount,
    reason: null,
  };
};

export const isDocumentQuadTrustworthy = (
  points: Point[] | null | undefined,
  frameWidth: number,
  frameHeight: number,
): boolean => {
  return assessDocumentQuad(points, frameWidth, frameHeight).trustworthy;
};

const GEOMETRY_GATE_MIN_EDGE_RATIO = 0.25;
const GEOMETRY_GATE_MIN_AREA_RATIO = 0.05;

export interface QuadGeometryValidation {
  valid: boolean;
  reason: string | null;
}

export const validateQuadGeometry = (
  points: Point[],
  imageWidth: number,
  imageHeight: number,
): QuadGeometryValidation => {
  if (points.length !== 4) {
    return {valid: false, reason: "Quad must have exactly 4 points."};
  }

  const [tl, tr, br, bl] = points;

  const top = distanceBetweenPoints(tl, tr);
  const right = distanceBetweenPoints(tr, br);
  const bottom = distanceBetweenPoints(bl, br);
  const left = distanceBetweenPoints(tl, bl);

  if (![top, right, bottom, left].every(Number.isFinite)) {
    return {valid: false, reason: "Quad has non-finite edge lengths."};
  }

  const horizontalRatio = Math.min(top, bottom) / Math.max(top, bottom, 1);
  if (horizontalRatio < GEOMETRY_GATE_MIN_EDGE_RATIO) {
    return {
      valid: false,
      reason: `Degenerate quad: horizontal edge ratio ${horizontalRatio.toFixed(3)} < ${GEOMETRY_GATE_MIN_EDGE_RATIO} (top=${top.toFixed(0)}, bottom=${bottom.toFixed(0)}).`,
    };
  }

  const verticalRatio = Math.min(left, right) / Math.max(left, right, 1);
  if (verticalRatio < GEOMETRY_GATE_MIN_EDGE_RATIO) {
    return {
      valid: false,
      reason: `Degenerate quad: vertical edge ratio ${verticalRatio.toFixed(3)} < ${GEOMETRY_GATE_MIN_EDGE_RATIO} (left=${left.toFixed(0)}, right=${right.toFixed(0)}).`,
    };
  }

  const quadArea = computePolygonArea(points);
  const imageArea = imageWidth * imageHeight;
  const areaRatio = quadArea / Math.max(imageArea, 1);
  if (areaRatio < GEOMETRY_GATE_MIN_AREA_RATIO) {
    return {
      valid: false,
      reason: `Quad too small: area ratio ${areaRatio.toFixed(3)} < ${GEOMETRY_GATE_MIN_AREA_RATIO}.`,
    };
  }

  if (!isConvexOrderedQuad([tl, tr, br, bl])) {
    return {valid: false, reason: "Quad is non-convex or self-intersecting."};
  }

  return {valid: true, reason: null};
};
