/// <reference lib="webworker" />

import {applyPerspectiveTransformToImageData} from "./perspective-transform";
import {enhanceDocumentImageData} from "./document-enhancer";
import type {OrthogonalRotation} from "./image-data";
import {refineDocumentQuadInImageData} from "./postprocess-corner-refinement";
import {validateQuadGeometry} from "./document-quad";
import {applyLocalSpineFlatteningToImageData} from "./postprocess-local-flattening";
import type {
  ScannerPostProcessWorkerErrorResponse,
  ScannerPostProcessWorkerProcessRequest,
  ScannerPostProcessWorkerRequest,
  ScannerPostProcessWorkerResponse,
} from "./scanner-postprocess-worker-protocol";

declare const self: DedicatedWorkerGlobalScope;

interface OpenCvWorkerRuntime {
  Mat?: unknown;
  onRuntimeInitialized?: (() => void) | null;
}

interface WorkerScopeWithOpenCv extends DedicatedWorkerGlobalScope {
  cv?: OpenCvWorkerRuntime;
}

type WorkerImageDecoder = {
  close?: () => void;
  decode: () => Promise<{ image: ImageBitmap }>;
};

type WorkerImageDecoderConstructor = new (init: {
  data: BufferSource;
  type: string;
}) => WorkerImageDecoder;

const OPEN_CV_INIT_TIMEOUT_MS = 12_000;
const workerScope = self as WorkerScopeWithOpenCv;
let openCvReadyPromise: Promise<void> | null = null;
let pngEncodeSurface: {
  canvas: OffscreenCanvas;
  context: OffscreenCanvasRenderingContext2D;
} | null = null;
let pngEncodeQueue: Promise<void> = Promise.resolve();

const postWorkerMessage = (
  message: ScannerPostProcessWorkerResponse,
  transfer: Transferable[] = [],
): void => {
  if (transfer.length > 0) {
    workerScope.postMessage(message, transfer);
    return;
  }

  workerScope.postMessage(message);
};

const postWorkerError = (payload: ScannerPostProcessWorkerErrorResponse): void => {
  postWorkerMessage(payload);
};

const getImageDecoderConstructorInWorker = (): WorkerImageDecoderConstructor | null => {
  const candidate = (workerScope as WorkerScopeWithOpenCv & {
    ImageDecoder?: WorkerImageDecoderConstructor;
  }).ImageDecoder;

  return typeof candidate === "function" ? candidate : null;
};

const bitmapToImageDataInWorker = (bitmap: ImageBitmap): ImageData => {
  const OffscreenCanvasConstructor = (workerScope as WorkerScopeWithOpenCv & {
    OffscreenCanvas?: typeof OffscreenCanvas;
  }).OffscreenCanvas;
  if (typeof OffscreenCanvasConstructor !== "function") {
    throw new Error("OffscreenCanvas is unavailable in the scanner post-process worker.");
  }

  const canvas = new OffscreenCanvasConstructor(bitmap.width, bitmap.height);
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Could not get an OffscreenCanvas 2D context in the scanner post-process worker.");
  }

  context.drawImage(bitmap, 0, 0);
  return context.getImageData(0, 0, bitmap.width, bitmap.height);
};

const getPngEncodeSurfaceInWorker = (
  width: number,
  height: number,
): {
  canvas: OffscreenCanvas;
  context: OffscreenCanvasRenderingContext2D;
} => {
  const OffscreenCanvasConstructor = (workerScope as WorkerScopeWithOpenCv & {
    OffscreenCanvas?: typeof OffscreenCanvas;
  }).OffscreenCanvas;
  if (typeof OffscreenCanvasConstructor !== "function") {
    throw new Error("OffscreenCanvas is unavailable in the scanner post-process worker.");
  }

  if (!pngEncodeSurface) {
    const canvas = new OffscreenCanvasConstructor(width, height);
    const context = canvas.getContext("2d");
    if (!context) {
      throw new Error("Could not get an OffscreenCanvas 2D context for post-process PNG encode.");
    }

    pngEncodeSurface = {
      canvas,
      context,
    };
  }

  if (pngEncodeSurface.canvas.width !== width) {
    pngEncodeSurface.canvas.width = width;
  }
  if (pngEncodeSurface.canvas.height !== height) {
    pngEncodeSurface.canvas.height = height;
  }

  return pngEncodeSurface;
};

const encodeImageDataToPngBytesInWorker = async (image: ImageData): Promise<ArrayBuffer> => {
  const previousEncode = pngEncodeQueue;
  let releaseEncode: (() => void) | undefined;
  pngEncodeQueue = new Promise<void>((resolve) => {
    releaseEncode = resolve;
  });

  await previousEncode;

  try {
    const surface = getPngEncodeSurfaceInWorker(image.width, image.height);
    surface.context.putImageData(image, 0, 0);
    const blob = await surface.canvas.convertToBlob({ type: "image/png" });
    return await blob.arrayBuffer();
  } finally {
    releaseEncode?.();
  }
};

const decodeBlobToImageDataInWorker = async (sourceBlob: Blob): Promise<ImageData> => {
  const ImageDecoderConstructor = getImageDecoderConstructorInWorker();
  if (ImageDecoderConstructor && sourceBlob.type) {
    const decoder = new ImageDecoderConstructor({
      data: await sourceBlob.arrayBuffer(),
      type: sourceBlob.type,
    });

    try {
      const { image } = await decoder.decode();
      try {
        return bitmapToImageDataInWorker(image);
      } finally {
        image.close();
      }
    } finally {
      decoder.close?.();
    }
  }

  if (typeof workerScope.createImageBitmap === "function") {
    const bitmap = await workerScope.createImageBitmap(sourceBlob);
    try {
      return bitmapToImageDataInWorker(bitmap);
    } finally {
      bitmap.close();
    }
  }

  throw new Error("Worker image decode is unavailable for scanner post-process.");
};

const ensureOpenCvReadyInWorker = async (): Promise<void> => {
  if (workerScope.cv?.Mat) {
    return;
  }

  if (openCvReadyPromise) {
    return await openCvReadyPromise;
  }

  openCvReadyPromise = new Promise<void>((resolve, reject) => {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;

    const cleanup = (): void => {
      if (timeoutId !== null) {
        clearTimeout(timeoutId);
      }
    };

    const resolveReady = (): void => {
      cleanup();
      resolve();
    };

    const rejectReady = (error: unknown): void => {
      cleanup();
      reject(error);
    };

    timeoutId = setTimeout(() => {
      rejectReady(new Error("Timed out while loading OpenCV in the scanner post-process worker."));
    }, OPEN_CV_INIT_TIMEOUT_MS);

    try {
      workerScope.importScripts("/opencv.js");
    } catch (error) {
      rejectReady(error);
      return;
    }

    const runtime = workerScope.cv;
    if (!runtime) {
      rejectReady(new Error("OpenCV did not expose a runtime in the scanner post-process worker."));
      return;
    }

    if (runtime.Mat) {
      resolveReady();
      return;
    }

    const previousHandler = runtime.onRuntimeInitialized;
    runtime.onRuntimeInitialized = () => {
      if (typeof previousHandler === "function") {
        previousHandler();
      }
      resolveReady();
    };
  });

  try {
    await openCvReadyPromise;
  } catch (error) {
    openCvReadyPromise = null;
    throw error;
  }
};

const rotateImageDataInWorker = (
  image: ImageData,
  rotation: OrthogonalRotation,
): ImageData => {
  if (rotation === 0) {
    return image;
  }

  const sourceWidth = image.width;
  const sourceHeight = image.height;
  const source = image.data;
  const targetWidth = rotation === 90 || rotation === 270 ? sourceHeight : sourceWidth;
  const targetHeight = rotation === 90 || rotation === 270 ? sourceWidth : sourceHeight;
  const target = new Uint8ClampedArray(targetWidth * targetHeight * 4);

  const writePixel = (targetX: number, targetY: number, sourceOffset: number): void => {
    const targetOffset = ((targetY * targetWidth) + targetX) * 4;
    target[targetOffset] = source[sourceOffset];
    target[targetOffset + 1] = source[sourceOffset + 1];
    target[targetOffset + 2] = source[sourceOffset + 2];
    target[targetOffset + 3] = source[sourceOffset + 3];
  };

  for (let sourceY = 0; sourceY < sourceHeight; sourceY += 1) {
    for (let sourceX = 0; sourceX < sourceWidth; sourceX += 1) {
      const sourceOffset = ((sourceY * sourceWidth) + sourceX) * 4;

      switch (rotation) {
        case 90:
          writePixel(targetWidth - 1 - sourceY, sourceX, sourceOffset);
          break;
        case 180:
          writePixel(targetWidth - 1 - sourceX, targetHeight - 1 - sourceY, sourceOffset);
          break;
        case 270:
          writePixel(sourceY, targetHeight - 1 - sourceX, sourceOffset);
          break;
        default:
          writePixel(sourceX, sourceY, sourceOffset);
          break;
      }
    }
  }

  return new ImageData(target, targetWidth, targetHeight);
};

const handleInit = async (): Promise<void> => {
  try {
    await ensureOpenCvReadyInWorker();
    postWorkerMessage({type: "ready"});
  } catch (error) {
    postWorkerError({
      type: "error",
      phase: "init",
      message: error instanceof Error ? error.message : String(error),
    });
  }
};

const handleProcess = async (message: ScannerPostProcessWorkerProcessRequest): Promise<void> => {
  try {
    await ensureOpenCvReadyInWorker();

    const startedAt = performance.now();
    let decodeMs: number | null = null;
    const imageData = message.inputKind === "encoded-image"
      ? await (async () => {
          if (!message.sourceBlob) {
            throw new Error("Encoded-image post-process request is missing sourceBlob.");
          }

          const decodeStartedAt = performance.now();
          const decodedImage = await decodeBlobToImageDataInWorker(message.sourceBlob);
          decodeMs = performance.now() - decodeStartedAt;
          return decodedImage;
        })()
      : (() => {
          if (
            typeof message.width !== "number"
            || typeof message.height !== "number"
            || !message.pixels
          ) {
            throw new Error("Image-data post-process request is missing width/height/pixels.");
          }

          return new ImageData(
            new Uint8ClampedArray(message.pixels),
            message.width,
            message.height,
          );
        })();
    let processedImage = imageData;
    let perspectiveMs: number | null = null;
    let refineMs: number | null = null;
    let flattenMs: number | null = null;
    let enhanceMs: number | null = null;
    const modelMs: number | null = null;
    const residualWarpMs: number | null = null;
    let rotateMs: number | null = null;
    let effectiveDocumentPoints = message.documentPoints;
    let refinementApplied = false;
    let localFlatteningApplied = false;
    const postprocessBackend = "heuristic";
    const modelId: string | null = null;
    const controlGridShape: string | null = null;
    const residualWarpApplied = false;
    const residualWarpFallbackReason = message.postprocessBackend === "native-ml-v1"
      ? "Worker fallback keeps the current heuristic stage-2 path; native residual-control-point inference is only attempted in Tauri."
      : null;

    if (message.documentPoints && message.documentPoints.length === 4) {
      const refineStartedAt = performance.now();
      const refinedQuad = refineDocumentQuadInImageData(imageData, message.documentPoints);
      refineMs = performance.now() - refineStartedAt;
      effectiveDocumentPoints = refinedQuad.points;
      refinementApplied = refinedQuad.applied;
    }

    if (effectiveDocumentPoints && effectiveDocumentPoints.length === 4) {
      const geometryCheck = validateQuadGeometry(effectiveDocumentPoints, imageData.width, imageData.height);
      if (!geometryCheck.valid) {
        effectiveDocumentPoints = null;
      }
    }

    if (effectiveDocumentPoints && effectiveDocumentPoints.length === 4) {
      const perspectiveStartedAt = performance.now();
      processedImage = applyPerspectiveTransformToImageData(imageData, effectiveDocumentPoints, 0.01);
      perspectiveMs = performance.now() - perspectiveStartedAt;
    }

    if (message.spineFlattening && effectiveDocumentPoints && effectiveDocumentPoints.length === 4) {
      const flattenStartedAt = performance.now();
      const flattenResult = applyLocalSpineFlatteningToImageData(processedImage);
      flattenMs = performance.now() - flattenStartedAt;
      processedImage = flattenResult.imageData;
      localFlatteningApplied = flattenResult.applied;
    }

    if (message.outputRotation !== 0) {
      const rotateStartedAt = performance.now();
      processedImage = rotateImageDataInWorker(processedImage, message.outputRotation);
      rotateMs = performance.now() - rotateStartedAt;
    }

    if (message.imageEnhancement) {
      const enhanceStartedAt = performance.now();
      processedImage = await enhanceDocumentImageData(processedImage, {
        preferSoftTone: localFlatteningApplied,
        colorMode: message.colorMode,
      });
      enhanceMs = performance.now() - enhanceStartedAt;
    }

    const encodeStartedAt = performance.now();
    const encodedBytes = await encodeImageDataToPngBytesInWorker(processedImage);
    const encodeMs = performance.now() - encodeStartedAt;

    postWorkerMessage({
      type: "result",
      requestId: message.requestId,
      processingMs: performance.now() - startedAt,
      decodeMs,
      refineMs,
      perspectiveMs,
      flattenMs,
      enhanceMs,
      modelMs,
      residualWarpMs,
      rotateMs,
      encodeMs,
      inputWidth: imageData.width,
      inputHeight: imageData.height,
      outputWidth: processedImage.width,
      outputHeight: processedImage.height,
      encodedMimeType: "image/jpeg",
      postprocessBackend,
      modelId,
      controlGridShape,
      effectiveDocumentPoints,
      refinementApplied,
      localFlatteningApplied,
      residualWarpApplied,
      residualWarpFallbackReason,
      encodedBytes,
    }, [encodedBytes]);
  } catch (error) {
    postWorkerError({
      type: "error",
      phase: "process",
      message: error instanceof Error ? error.message : String(error),
      requestId: message.requestId,
    });
  }
};

workerScope.addEventListener("message", (event: MessageEvent<ScannerPostProcessWorkerRequest>) => {
  switch (event.data.type) {
    case "init":
      void handleInit();
      break;
    case "process":
      void handleProcess(event.data);
      break;
    default:
      postWorkerError({
        type: "error",
        phase: "runtime",
        message: `Unsupported scanner post-process worker message: ${(event.data as {type?: string}).type ?? "unknown"}`,
      });
      break;
  }
});

export {};
