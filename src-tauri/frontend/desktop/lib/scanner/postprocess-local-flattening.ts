/**
 * Page flattening v9: Expanded Geometric Mesh & Secondary Crop
 *
 * Physical model: Phase 1 provides an expanded rectangular canvas containing the
 * complete physical page (margins are pure black).
 * By directly extracting the continuous top and bottom boundary curves of this page,
 * we inherently capture both horizontal paper curl foreshortening and
 * vertical perspective pitch sag.
 *
 * We map these curves flat to produce the final cropped, straight output.
 */

const SAT_REJECT = 0.25;
const COLOR_SAT_MAX = 0.15;
const COLOR_VAL_MIN = 0.6;
const COLOR_SPREAD_MAX = 50;

export interface LocalFlatteningResult {
  imageData: ImageData;
  applied: boolean;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

const toGrayscale = (img: ImageData): Float32Array => {
  const out = new Float32Array(img.width * img.height);
  for (let i = 0; i < out.length; i++) {
    const o = i * 4;
    out[i] = img.data[o] * 0.299 + img.data[o + 1] * 0.587 + img.data[o + 2] * 0.114;
  }
  return out;
};

const computeOtsuThreshold = (gray: Float32Array): number => {
  const hist = new Int32Array(256);
  for (let i = 0; i < gray.length; i++) {
    hist[clamp(Math.round(gray[i]), 0, 255)]++;
  }
  const total = gray.length;
  let sumAll = 0;
  for (let i = 0; i < 256; i++) sumAll += i * hist[i];

  let wB = 0, sumB = 0, maxVar = 0, threshold = 128;
  for (let t = 0; t < 256; t++) {
    wB += hist[t];
    if (wB === 0) continue;
    const wF = total - wB;
    if (wF === 0) break;
    sumB += t * hist[t];
    const mB = sumB / wB;
    const mF = (sumAll - sumB) / wF;
    const v = wB * wF * (mB - mF) * (mB - mF);
    if (v > maxVar) { maxVar = v; threshold = t; }
  }
  return threshold;
};

const buildPaperMask = (
  gray: Float32Array, img: ImageData, threshold: number,
): Uint8Array => {
  const n = img.width * img.height;
  const mask = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const o = i * 4;
    const r = img.data[o], g = img.data[o + 1], b = img.data[o + 2];
    const maxC = Math.max(r, g, b);
    const minC = Math.min(r, g, b);
    const sat = maxC > 0 ? (maxC - minC) / maxC : 0;
    const grayOk = gray[i] > threshold && sat < SAT_REJECT;
    const colorOk = (maxC / 255) > COLOR_VAL_MIN
      && sat < COLOR_SAT_MAX
      && (maxC - minC) < COLOR_SPREAD_MAX;
    mask[i] = (grayOk || colorOk) ? 1 : 0;
  }
  return mask;
};

const gaussianSmooth1D = (arr: (number | null)[], sigma: number, fallback: number): Float32Array => {
  const valid = arr.map(v => v === null ? fallback : v);

  // Impute missing edges from nearest valid
  let last = fallback;
  for (let i = 0; i < valid.length; i++) {
    if (arr[i] !== null) { last = arr[i]!; }
    valid[i] = last;
  }
  last = fallback;
  for (let i = valid.length - 1; i >= 0; i--) {
    if (arr[i] !== null) { last = arr[i]!; }
    valid[i] = last;
  }

  const out = new Float32Array(valid.length);
  const radius = Math.ceil(sigma * 3);
  const kernel: number[] = [];
  let kSum = 0;
  for (let k = -radius; k <= radius; k++) {
    const v = Math.exp(-0.5 * (k / sigma) * (k / sigma));
    kernel.push(v);
    kSum += v;
  }
  for (let i = 0; i < kernel.length; i++) kernel[i] /= kSum;
  for (let i = 0; i < valid.length; i++) {
    let sum = 0;
    for (let k = -radius; k <= radius; k++) {
      sum += valid[clamp(i + k, 0, valid.length - 1)] * kernel[k + radius];
    }
    out[i] = sum;
  }
  return out;
};

const sampleBilinear = (
  data: Uint8ClampedArray, w: number, h: number, x: number, y: number,
): [number, number, number, number] => {
  const cx = clamp(x, 0, w - 1), cy = clamp(y, 0, h - 1);
  const x0 = Math.floor(cx), y0 = Math.floor(cy);
  const x1 = Math.min(w - 1, x0 + 1), y1 = Math.min(h - 1, y0 + 1);
  const tx = cx - x0, ty = cy - y0;

  const oTL = (y0 * w + x0) * 4;
  const tlR = data[oTL], tlG = data[oTL + 1], tlB = data[oTL + 2], tlA = data[oTL + 3];

  const oTR = (y0 * w + x1) * 4;
  const trR = data[oTR], trG = data[oTR + 1], trB = data[oTR + 2], trA = data[oTR + 3];

  const oBL = (y1 * w + x0) * 4;
  const blR = data[oBL], blG = data[oBL + 1], blB = data[oBL + 2], blA = data[oBL + 3];

  const oBR = (y1 * w + x1) * 4;
  const brR = data[oBR], brG = data[oBR + 1], brB = data[oBR + 2], brA = data[oBR + 3];

  const tR = tlR + (trR - tlR) * tx;
  const bR = blR + (brR - blR) * tx;
  const r = tR + (bR - tR) * ty;

  const tG = tlG + (trG - tlG) * tx;
  const bG = blG + (brG - blG) * tx;
  const g = tG + (bG - tG) * ty;

  const tB = tlB + (trB - tlB) * tx;
  const bB = blB + (brB - blB) * tx;
  const b = tB + (bB - tB) * ty;

  const tA = tlA + (trA - tlA) * tx;
  const bA = blA + (brA - blA) * tx;
  const a = tA + (bA - tA) * ty;

  return [Math.round(r), Math.round(g), Math.round(b), Math.round(a)];
};

export const applyLocalSpineFlatteningToImageData = (
  imageData: ImageData
): LocalFlatteningResult => {
  const { width, height } = imageData;
  if (width < 48 || height < 48) return { imageData, applied: false };

  // 1. Threshold & Build Mask
  const gray = toGrayscale(imageData);
  const threshold = computeOtsuThreshold(gray);
  const mask = buildPaperMask(gray, imageData, threshold);

  // 2. Locate the precise horizontal boundaries of the padded page
  let paperMinX = -1, paperMaxX = -1;
  const colThreshold = Math.floor(height * 0.10);

  for (let x = 0; x < width; x++) {
    let colSum = 0;
    for (let y = 0; y < height; y++) { if (mask[y * width + x]) colSum++; }
    if (colSum > colThreshold) { paperMinX = x; break; }
  }
  for (let x = width - 1; x >= 0; x--) {
    let colSum = 0;
    for (let y = 0; y < height; y++) { if (mask[y * width + x]) colSum++; }
    if (colSum > colThreshold) { paperMaxX = x; break; }
  }

  // Margin sanity check
  if (paperMinX < 0 || paperMaxX < 0 || paperMaxX - paperMinX < 20) {
    return { imageData, applied: false };
  }

  // Add a tiny inner margin to avoid tracking the messy physical tear/noise edge perfectly
  const innerPad = 5;
  const safeMinX = clamp(paperMinX + innerPad, 0, width - 1);
  const safeMaxX = clamp(paperMaxX - innerPad, safeMinX, width - 1);
  const paperW = safeMaxX - safeMinX + 1;

  // 3. Trace Top & Bottom physical boundaries natively
  const rawTop: (number | null)[] = new Array(paperW).fill(null);
  const rawBottom: (number | null)[] = new Array(paperW).fill(null);
  const midY = Math.floor(height / 2);

  for (let i = 0; i < paperW; i++) {
    const x = safeMinX + i;
    for (let y = 0; y < midY; y++) {
      if (mask[y * width + x]) { rawTop[i] = y; break; }
    }
    for (let y = height - 1; y >= midY; y--) {
      if (mask[y * width + x]) { rawBottom[i] = y; break; }
    }
  }

  // 4. Extract smoothed geometric curves
  // Book curves are continuous; heavy Gaussian rejects text/thumbs noise
  const SMOOTH_SIGMA = Math.max(15, Math.floor(paperW * 0.05));
  const topCurve = gaussianSmooth1D(rawTop, SMOOTH_SIGMA, midY / 2);
  const bottomCurve = gaussianSmooth1D(rawBottom, SMOOTH_SIGMA, height - midY / 2);

  // 5. Measure physical foreshortening H(x) & base projection
  const H = new Float32Array(paperW);
  for (let i = 0; i < paperW; i++) {
    H[i] = Math.max(1, bottomCurve[i] - topCurve[i]);
  }

  const sortedH = Array.from(H).sort((a, b) => a - b);
  const hFlat = sortedH[Math.floor(paperW * 0.90)]; // 90th percentile is the un-curled flat dimension
  const minH = sortedH[0];

  // If variation is < 4%, the image is already flat.
  if (minH > hFlat * 0.96) {
    return { imageData, applied: false };
  }

  // 6. Compute Unrolled Width using stretch integral
  const stretch = new Float32Array(paperW);
  let totalOutputWidth = 0;
  for (let i = 0; i < paperW; i++) {
    const s = Math.max(1.0, hFlat / Math.max(1, H[i]));
    stretch[i] = Math.min(s, 2.5); // restrict extreme stretching
    totalOutputWidth += stretch[i];
  }

  const outW = Math.round(totalOutputWidth);
  const outH = Math.round(hFlat);

  if (outW <= 0 || outH <= 0 || outW > width * 3 || outH > height * 3) {
    return { imageData, applied: false };
  }

  // Integral Mapping: Destination Flat X -> Source Curled X
  const dstXtoSrcX = new Float32Array(outW);
  let currentAccum = 0;
  let srcInt = 0;
  for (let dstX = 0; dstX < outW; dstX++) {
    while (srcInt < paperW - 1 && currentAccum + stretch[srcInt] < dstX) {
      currentAccum += stretch[srcInt];
      srcInt++;
    }
    const fractional = srcInt < paperW - 1 ? (dstX - currentAccum) / stretch[srcInt] : 0;
    dstXtoSrcX[dstX] = Math.max(0, Math.min(paperW - 1, srcInt + fractional));
  }

  // 7. Render flat secondary crop
  const outData = new Uint8ClampedArray(outW * outH * 4);
  const srcPixels = imageData.data;

  for (let dx = 0; dx < outW; dx++) {
    const fx = dstXtoSrcX[dx];
    const ix = Math.floor(fx);
    const tx = fx - ix;
    const nx = Math.min(paperW - 1, ix + 1);

    // Linearly interpolate exactly on the continuous curve
    const t0 = topCurve[ix]; const t1 = topCurve[nx];
    const topY = t0 + (t1 - t0) * tx;

    const b0 = bottomCurve[ix]; const b1 = bottomCurve[nx];
    const botY = b0 + (b1 - b0) * tx;

    const currentH = botY - topY;
    const sx = safeMinX + fx;

    for (let dy = 0; dy < outH; dy++) {
       // Inverse perspective mapping:
       // linearly interpolating between exactly topCurve and bottomCurve
       // completely removes orthographic pitch errors.
       const yRatio = dy / (outH - 1);
       const sy = topY + yRatio * currentH;

       const pixel = sampleBilinear(srcPixels, width, height, sx, sy);
       const o = (dy * outW + dx) * 4;
       outData[o] = pixel[0];
       outData[o+1] = pixel[1];
       outData[o+2] = pixel[2];
       outData[o+3] = pixel[3];
    }
  }

  return {
    imageData: new ImageData(outData, outW, outH),
    applied: true
  };
};
