/**
 * Binary frame packet parsing and I420 decoding utilities for the scanner.
 */

export const FRAME_CODEC_I420 = 3;
export const FRAME_CODEC_I420_TELEMETRY = 4;
export const FRAME_PACKET_HEADER_SIZE = 9;
export const FRAME_PACKET_TELEMETRY_SIZE = 12;

export interface NormalizedFramePayload { buffer: ArrayBuffer; byteOffset: number; byteLength: number; }
export interface FramePacketTelemetry { sentAtEpochMs: number; sequence: number; }
export interface ParsedFramePacket { codec: number; width: number; height: number; payload: Uint8Array; telemetry: FramePacketTelemetry | null; }
export interface DecodedRgbaFrame { width: number; height: number; rgba: Uint8ClampedArray; telemetry: FramePacketTelemetry | null; }


const IS_LITTLE_ENDIAN = new Uint8Array(new Uint32Array([0x11223344]).buffer)[0] === 0x44;
const Y_TO_RGB_LUT = new Int32Array(256);
const U_TO_BLUE_LUT = new Int32Array(256);
const U_TO_GREEN_LUT = new Int32Array(256);
const V_TO_RED_LUT = new Int32Array(256);
const V_TO_GREEN_LUT = new Int32Array(256);
 
const LITTLE_ENDIAN_RGBA_ALPHA = 0xff << 24;
 
const BIG_ENDIAN_RGBA_ALPHA = 0xff;

for (let value = 0; value < 256; value += 1) {

  const luma = Math.max(0, value - 16);
  const chroma = value - 128;
  Y_TO_RGB_LUT[value] = 298 * luma;
  U_TO_BLUE_LUT[value] = 516 * chroma;
  U_TO_GREEN_LUT[value] = -100 * chroma;
  V_TO_RED_LUT[value] = 409 * chroma;
  V_TO_GREEN_LUT[value] = -208 * chroma;
}

/**
 * Normalize different Tauri IPC payload shapes to a single buffer view.
 *
 * @param {Uint8Array | ArrayBuffer | number[]} data
 * @returns {{ buffer: ArrayBuffer, byteOffset: number, byteLength: number }}
 */
export const normalizeFramePayload = (data: Uint8Array | ArrayBuffer | number[]): NormalizedFramePayload => {
  if (data instanceof ArrayBuffer) {
    return {
      buffer: data,
      byteOffset: 0,
      byteLength: data.byteLength,
    };
  }

  if (ArrayBuffer.isView(data)) {
    return {
      buffer: data.buffer as ArrayBuffer,
      byteOffset: data.byteOffset,
      byteLength: data.byteLength,
    };
  }

  const normalized = Uint8Array.from(data);
  return {
    buffer: normalized.buffer as ArrayBuffer,
    byteOffset: 0,
    byteLength: normalized.byteLength,
  };
};

/**
 * Parse a codec-tagged scanner frame packet.
 *
 * @param {Uint8Array | ArrayBuffer | number[]} data
 * @returns {{ codec: number, width: number, height: number, payload: Uint8Array }}
 */
export const parseFramePacket = (data: Uint8Array | ArrayBuffer | number[]): ParsedFramePacket => {
  const { buffer, byteOffset, byteLength } = normalizeFramePayload(data);

  if (byteLength < FRAME_PACKET_HEADER_SIZE) {
    throw new Error("Frame packet is shorter than the protocol header.");
  }

  const view = new DataView(buffer, byteOffset, byteLength);
  const codec = view.getUint8(0);
  const width = view.getUint32(1, false);
  const height = view.getUint32(5, false);
  let payloadOffset = FRAME_PACKET_HEADER_SIZE;
  let telemetry: FramePacketTelemetry | null = null;

  if (codec === FRAME_CODEC_I420_TELEMETRY) {
    if (byteLength < FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE) {
      throw new Error("Frame packet telemetry header is truncated.");
    }

    telemetry = {
      sentAtEpochMs: Number(view.getBigUint64(FRAME_PACKET_HEADER_SIZE, false)),
      sequence: view.getUint32(FRAME_PACKET_HEADER_SIZE + 8, false),
    };
    payloadOffset += FRAME_PACKET_TELEMETRY_SIZE;
  }

  const payload = new Uint8Array(buffer, byteOffset + payloadOffset, byteLength - payloadOffset);

  return { codec, width, height, payload, telemetry };
};

/**
 * Decode a codec-tagged frame packet to RGBA pixels.
 *
 * @param {Uint8Array | ArrayBuffer | number[]} data
 * @param {Uint8ClampedArray=} targetRgba
 * @returns {{ width: number, height: number, rgba: Uint8ClampedArray }}
 */
export const decodeFramePacketToRgba = (data: Uint8Array | ArrayBuffer | number[], targetRgba?: Uint8ClampedArray): DecodedRgbaFrame => {
  const { codec, width, height, payload, telemetry } = parseFramePacket(data);

  if (codec === FRAME_CODEC_I420 || codec === FRAME_CODEC_I420_TELEMETRY) {
    return {
      width,
      height,
      rgba: decodeI420ToRgba(payload, width, height, targetRgba),
      telemetry,
    };
  }

  throw new Error(`Unsupported scanner frame codec: ${codec}.`);
};



/**
 * Decode a tightly packed I420 frame to RGBA.
 *
 * @param {Uint8Array} payload
 * @param {number} width
 * @param {number} height
 * @param {Uint8ClampedArray=} targetRgba
 * @returns {Uint8ClampedArray}
 */
export const decodeI420ToRgba = (payload: Uint8Array, width: number, height: number, targetRgba?: Uint8ClampedArray): Uint8ClampedArray => {
  if ((width & 1) !== 0 || (height & 1) !== 0) {
    throw new Error(`I420 preview frames require even dimensions, got ${width}x${height}.`);
  }

  const lumaSize = width * height;
  const chromaWidth = width >> 1;
  const chromaHeight = height >> 1;
  const chromaSize = chromaWidth * chromaHeight;
  const expectedSize = lumaSize + chromaSize * 2;

  if (payload.length !== expectedSize) {
    throw new Error(`Invalid I420 payload size: expected ${expectedSize}, got ${payload.length}.`);
  }

  const yPlane = payload.subarray(0, lumaSize);
  const uPlane = payload.subarray(lumaSize, lumaSize + chromaSize);
  const vPlane = payload.subarray(lumaSize + chromaSize, expectedSize);
  const { rgba, rgba32 } = resolveRgbaTarget(lumaSize, targetRgba);

  for (let row = 0; row < height; row += 2) {
    const yRow0 = row * width;
    const yRow1 = yRow0 + width;
    const chromaRow = (row >> 1) * chromaWidth;

    for (let col = 0; col < width; col += 2) {
      const chromaIndex = chromaRow + (col >> 1);
      const u = uPlane[chromaIndex];
      const v = vPlane[chromaIndex];
      const blueContribution = U_TO_BLUE_LUT[u];
      const greenContribution = U_TO_GREEN_LUT[u] + V_TO_GREEN_LUT[v];
      const redContribution = V_TO_RED_LUT[v];

      const pixel0 = yRow0 + col;
      rgba32[pixel0] = packRgbaFromYuv(
        yPlane[pixel0],
        redContribution,
        greenContribution,
        blueContribution,
      );
      rgba32[pixel0 + 1] = packRgbaFromYuv(
        yPlane[pixel0 + 1],
        redContribution,
        greenContribution,
        blueContribution,
      );

      const pixel2 = yRow1 + col;
      rgba32[pixel2] = packRgbaFromYuv(
        yPlane[pixel2],
        redContribution,
        greenContribution,
        blueContribution,
      );
      rgba32[pixel2 + 1] = packRgbaFromYuv(
        yPlane[pixel2 + 1],
        redContribution,
        greenContribution,
        blueContribution,
      );
    }
  }

  return rgba;
};

/**
 * Resolve a reusable RGBA target buffer for decoder hot paths.
 *
 * @param {number} pixelCount
 * @param {Uint8ClampedArray=} targetRgba
 * @returns {{ rgba: Uint8ClampedArray, rgba32: Uint32Array }}
 */
const resolveRgbaTarget = (pixelCount: number, targetRgba?: Uint8ClampedArray): { rgba: Uint8ClampedArray, rgba32: Uint32Array } => {
  const byteLength = pixelCount * 4;

  if (
    targetRgba &&
    targetRgba.length === byteLength &&
    (targetRgba.byteOffset & 0x03) === 0
  ) {
    return {
      rgba: targetRgba,
      rgba32: new Uint32Array(targetRgba.buffer, targetRgba.byteOffset, pixelCount),
    };
  }

  const rgba = new Uint8ClampedArray(byteLength);
  return {
    rgba,
    rgba32: new Uint32Array(rgba.buffer, rgba.byteOffset, pixelCount),
  };
};

/**
 * Pack one YUV pixel into RGBA32.
 *
 * @param {number} y
 * @param {number} redContribution
 * @param {number} greenContribution
 * @param {number} blueContribution
 * @returns {number}
 */
const packRgbaFromYuv = (y: number, redContribution: number, greenContribution: number, blueContribution: number): number => {
  const base = Y_TO_RGB_LUT[y];
  const red = clampByte((base + redContribution + 128) >> 8);
  const green = clampByte((base + greenContribution + 128) >> 8);
  const blue = clampByte((base + blueContribution + 128) >> 8);

  if (IS_LITTLE_ENDIAN) {
    return LITTLE_ENDIAN_RGBA_ALPHA | (blue << 16) | (green << 8) | red;
  }

  return (red << 24) | (green << 16) | (blue << 8) | BIG_ENDIAN_RGBA_ALPHA;
};

/**
 * Clamp an integer channel value to the 0..255 range.
 *
 * @param {number} value
 * @returns {number}
 */
const clampByte = (value: number): number => {
  if (value < 0) return 0;
  if (value > 255) return 255;
  return value;
};
