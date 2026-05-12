const MIN_COMPONENT_AREA_RATIO = 0.12;
const MIN_BBOX_AREA_RATIO = 0.18;
const SKIP_IF_NEAR_FULL_RATIO = 0.985;
const MARGIN_RATIO = 0.02;
const MIN_MARGIN_PX = 4;
const MIN_SCORE_THRESHOLD = 160;
const PAPER_SCORE_BLUR_RADIUS = 6;
const PAPER_SCORE_LOCAL_CONTRAST_WEIGHT = 1.15;

interface BoundingBox {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export interface PaperCropResult {
  imageData: ImageData;
  applied: boolean;
}

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const blurGray = (
  gray: Uint8Array,
  width: number,
  height: number,
  radius: number,
): Uint8Array => {
  if (radius <= 0) {
    return gray.slice();
  }

  const horizontal = new Float32Array(width * height);
  const output = new Uint8Array(width * height);

  for (let y = 0; y < height; y += 1) {
    let sum = 0;
    for (let dx = -radius; dx <= radius; dx += 1) {
      const clampedX = clamp(dx, 0, width - 1);
      sum += gray[(y * width) + clampedX];
    }

    for (let x = 0; x < width; x += 1) {
      horizontal[(y * width) + x] = sum / ((radius * 2) + 1);
      const removeX = clamp(x - radius, 0, width - 1);
      const addX = clamp(x + radius + 1, 0, width - 1);
      sum += gray[(y * width) + addX] - gray[(y * width) + removeX];
    }
  }

  for (let x = 0; x < width; x += 1) {
    let sum = 0;
    for (let dy = -radius; dy <= radius; dy += 1) {
      const clampedY = clamp(dy, 0, height - 1);
      sum += horizontal[(clampedY * width) + x];
    }

    for (let y = 0; y < height; y += 1) {
      output[(y * width) + x] = clamp(
        Math.round(sum / ((radius * 2) + 1)),
        0,
        255,
      );
      const removeY = clamp(y - radius, 0, height - 1);
      const addY = clamp(y + radius + 1, 0, height - 1);
      sum += horizontal[(addY * width) + x] - horizontal[(removeY * width) + x];
    }
  }

  return output;
};

const buildPaperScoreData = (imageData: ImageData): Uint8Array => {
  const gray = new Uint8Array(imageData.width * imageData.height);
  for (let index = 0; index < gray.length; index += 1) {
    const offset = index * 4;
    const red = imageData.data[offset];
    const green = imageData.data[offset + 1];
    const blue = imageData.data[offset + 2];
    gray[index] = clamp(
      Math.round((red * 0.299) + (green * 0.587) + (blue * 0.114)),
      0,
      255,
    );
  }

  const blurredGray = blurGray(gray, imageData.width, imageData.height, PAPER_SCORE_BLUR_RADIUS);
  const scores = new Uint8Array(imageData.width * imageData.height);

  for (let index = 0; index < scores.length; index += 1) {
    const offset = index * 4;
    const red = imageData.data[offset];
    const green = imageData.data[offset + 1];
    const blue = imageData.data[offset + 2];
    const maxChannel = Math.max(red, green, blue);
    const minChannel = Math.min(red, green, blue);
    const saturation = maxChannel - minChannel;
    const localContrast = Math.abs(gray[index] - blurredGray[index]);
    scores[index] = clamp(
      Math.round(
        blurredGray[index]
        - (saturation * 0.9)
        - (localContrast * PAPER_SCORE_LOCAL_CONTRAST_WEIGHT),
      ),
      0,
      255,
    );
  }

  return scores;
};

const otsuThreshold = (scores: Uint8Array): number => {
  const histogram = new Uint32Array(256);
  for (const score of scores) {
    histogram[score] += 1;
  }

  const total = scores.length;
  let sum = 0;
  for (let value = 0; value < histogram.length; value += 1) {
    sum += value * histogram[value];
  }

  let backgroundWeight = 0;
  let backgroundSum = 0;
  let bestThreshold = 0;
  let bestVariance = -1;

  for (let value = 0; value < histogram.length; value += 1) {
    backgroundWeight += histogram[value];
    if (backgroundWeight === 0) {
      continue;
    }

    const foregroundWeight = total - backgroundWeight;
    if (foregroundWeight === 0) {
      break;
    }

    backgroundSum += value * histogram[value];
    const backgroundMean = backgroundSum / backgroundWeight;
    const foregroundMean = (sum - backgroundSum) / foregroundWeight;
    const variance = backgroundWeight
      * foregroundWeight
      * (backgroundMean - foregroundMean)
      * (backgroundMean - foregroundMean);

    if (variance > bestVariance) {
      bestVariance = variance;
      bestThreshold = value;
    }
  }

  return Math.max(bestThreshold, MIN_SCORE_THRESHOLD);
};

const largestPaperComponentBox = (
  scores: Uint8Array,
  width: number,
  height: number,
  threshold: number,
): BoundingBox | null => {
  const minComponentArea = Math.round(width * height * MIN_COMPONENT_AREA_RATIO);
  const visited = new Uint8Array(scores.length);
  let bestBox: BoundingBox | null = null;
  let bestScore = Number.NEGATIVE_INFINITY;

  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const startIndex = (y * width) + x;
      if (visited[startIndex] === 1 || scores[startIndex] < threshold) {
        continue;
      }

      const queueX: number[] = [x];
      const queueY: number[] = [y];
      visited[startIndex] = 1;
      let cursor = 0;
      let area = 0;
      let scoreSum = 0;
      const box: BoundingBox = {minX: x, minY: y, maxX: x, maxY: y};

      while (cursor < queueX.length) {
        const currentX = queueX[cursor];
        const currentY = queueY[cursor];
        cursor += 1;
        const index = (currentY * width) + currentX;

        area += 1;
        scoreSum += scores[index];
        box.minX = Math.min(box.minX, currentX);
        box.minY = Math.min(box.minY, currentY);
        box.maxX = Math.max(box.maxX, currentX);
        box.maxY = Math.max(box.maxY, currentY);

        for (let dy = -1; dy <= 1; dy += 1) {
          for (let dx = -1; dx <= 1; dx += 1) {
            if (dx === 0 && dy === 0) {
              continue;
            }

            const nx = currentX + dx;
            const ny = currentY + dy;
            if (nx < 0 || ny < 0 || nx >= width || ny >= height) {
              continue;
            }

            const neighborIndex = (ny * width) + nx;
            if (visited[neighborIndex] === 1 || scores[neighborIndex] < threshold) {
              continue;
            }

            visited[neighborIndex] = 1;
            queueX.push(nx);
            queueY.push(ny);
          }
        }
      }

      if (area < minComponentArea) {
        continue;
      }

      const boxWidth = box.maxX - box.minX + 1;
      const boxHeight = box.maxY - box.minY + 1;
      const meanScore = scoreSum / area / 255;
      const fillRatio = area / Math.max(1, boxWidth * boxHeight);
      const componentScore = area * (meanScore + fillRatio);

      if (componentScore > bestScore) {
        bestScore = componentScore;
        bestBox = box;
      }
    }
  }

  return bestBox;
};

const cropImageData = (
  imageData: ImageData,
  box: BoundingBox,
): ImageData => {
  const boxWidth = box.maxX - box.minX + 1;
  const boxHeight = box.maxY - box.minY + 1;
  const marginX = Math.max(MIN_MARGIN_PX, Math.round(boxWidth * MARGIN_RATIO));
  const marginY = Math.max(MIN_MARGIN_PX, Math.round(boxHeight * MARGIN_RATIO));
  const cropX = Math.max(0, box.minX - marginX);
  const cropY = Math.max(0, box.minY - marginY);
  const cropMaxX = Math.min(imageData.width - 1, box.maxX + marginX);
  const cropMaxY = Math.min(imageData.height - 1, box.maxY + marginY);
  const cropWidth = cropMaxX - cropX + 1;
  const cropHeight = cropMaxY - cropY + 1;
  const output = new Uint8ClampedArray(cropWidth * cropHeight * 4);

  for (let y = 0; y < cropHeight; y += 1) {
    const sourceStart = (((cropY + y) * imageData.width) + cropX) * 4;
    const sourceEnd = sourceStart + (cropWidth * 4);
    output.set(imageData.data.subarray(sourceStart, sourceEnd), y * cropWidth * 4);
  }

  return new ImageData(output, cropWidth, cropHeight);
};

export const cropToPaperRegionInImageData = (imageData: ImageData): PaperCropResult => {
  if (imageData.width < 48 || imageData.height < 48) {
    return {
      imageData,
      applied: false,
    };
  }

  const scores = buildPaperScoreData(imageData);
  const threshold = otsuThreshold(scores);
  const box = largestPaperComponentBox(scores, imageData.width, imageData.height, threshold);
  if (!box) {
    return {
      imageData,
      applied: false,
    };
  }

  const boxAreaRatio = ((box.maxX - box.minX + 1) * (box.maxY - box.minY + 1))
    / (imageData.width * imageData.height);
  if (boxAreaRatio < MIN_BBOX_AREA_RATIO || boxAreaRatio >= SKIP_IF_NEAR_FULL_RATIO) {
    return {
      imageData,
      applied: false,
    };
  }

  const cropped = cropImageData(imageData, box);
  if (cropped.width >= imageData.width && cropped.height >= imageData.height) {
    return {
      imageData,
      applied: false,
    };
  }

  return {
    imageData: cropped,
    applied: true,
  };
};
