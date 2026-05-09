import {useCallback, useEffect, useRef, useState} from "react";
import type {Dispatch, MutableRefObject, SetStateAction} from "react";
import {useTranslation} from "react-i18next";
import {toast} from "sonner";

import {
  evaluateFrameMappingCompatibility,
  scalePointsBetweenFrames,
  type FrameSource,
  type Point,
  type ScannerStillCapture,
} from "@/lib/scanner";
import {
  decodeBlobToImageData,
  type OrthogonalRotation,
} from "@/lib/scanner/image-data";
import {mapPointsFromRotatedFrameToSource, type PreviewOrientation} from "@/lib/scanner/preview-orientation";
import {assessDocumentQuad, isDocumentQuadTrustworthy} from "@/lib/scanner/document-quad";
import type {PostProcessOptions} from "@/components/scanner/ScannerCapturedDocumentEditor";
import {detectDocumentWithTauriNativeOrt} from "@/lib/tauri/scanner-detect";
import {processTauriScannerPostProcessSourceFile} from "@/lib/tauri/scanner";
import {
  type ScannerCapturedDocument,
  useScannerStore,
} from "@/store/scanner-store";
import {
  type ScannerDetectionBackend,
  type ScannerPostProcessBackend,
  useSettingsStore,
} from "@/store/settings-store";

const PREVIEW_CAPTURE_COOLDOWN_MS = 1200;
const CAPTURE_CV_MAX_WIDTH = 1024;
const CAPTURE_CV_MAX_HEIGHT = 1024;
const STILL_CAPTURE_ROTATION_CANDIDATES: readonly OrthogonalRotation[] = [0, 90, 270, 180] as const;

interface PreviewCaptureSnapshot {
  frame: ImageData;
  points: Point[] | null;
}

interface PreparedCaptureArtifact {
  sourceBlob: Blob;
  sourceWidth: number;
  sourceHeight: number;
  points: Point[] | null;
  outputRotation: OrthogonalRotation;
  source?: string;
}

interface ProcessedDocumentRenderResult {
  blob: Blob;
  decodeMs: number | null;
  refineMs: number | null;
  inputWidth: number;
  inputHeight: number;
  outputWidth: number;
  outputHeight: number;
  perspectiveMs: number | null;
  flattenMs: number | null;
  enhanceMs: number | null;
  modelMs: number | null;
  residualWarpMs: number | null;
  rotateMs: number | null;
  encodeMs: number;
  postprocessBackend: ScannerPostProcessBackend;
  modelId: string | null;
  controlGridShape: string | null;
  effectiveDocumentPoints: Point[] | null;
  refinementApplied: boolean;
  localFlatteningApplied: boolean;
  residualWarpApplied: boolean;
  residualWarpFallbackReason: string | null;
}

export interface UseScannerCaptureParams {
  dialogOpenRef: MutableRefObject<boolean>;
  frameSourceRef: MutableRefObject<FrameSource | null>;
  activeDetectionBackendRef: MutableRefObject<ScannerDetectionBackend>;
  getLatestFrame: () => ImageData | null;
  captureProcessingRef?: MutableRefObject<boolean>;
  orientationRef: MutableRefObject<PreviewOrientation>;
}

export interface UseScannerCaptureResult {
  capture: () => Promise<void>;
  requestAutoCapture: (frame: ImageData, points: Point[] | null) => void;
  autoCapture: boolean;
  setAutoCapture: Dispatch<SetStateAction<boolean>>;
  isCapturing: boolean;
  isCaptureCommitPending: boolean;
  capturedDocuments: ScannerCapturedDocument[];
  removeCapturedDocument: (documentId: string) => void;
  reprocessCapturedDocument: (
    documentId: string,
    nextPoints: Point[],
    options?: PostProcessOptions,
  ) => void;
  getEditorPreview: (
    doc: ScannerCapturedDocument,
    points: Point[],
    options: PostProcessOptions,
  ) => Promise<{ blob: Blob; processingMs: number }>;
  invalidatePendingCommits: () => void;
  invalidateQueue: () => void;
  resetCaptureRuntime: () => void;
  resetCaptureDebug: () => void;
}

const cloneFrame = (frame: ImageData): ImageData => {
  return new ImageData(new Uint8ClampedArray(frame.data), frame.width, frame.height);
};

const clonePoints = (points: Point[] | null): Point[] | null => {
  return points?.map((point) => ({...point})) ?? null;
};

const describeHexWindow = (bytes: Uint8Array, count: number, fromEnd: boolean = false): string => {
  if (bytes.byteLength === 0) {
    return "—";
  }

  const safeCount = Math.max(1, Math.min(count, bytes.byteLength));
  const slice = fromEnd
    ? bytes.slice(bytes.byteLength - safeCount)
    : bytes.slice(0, safeCount);
  return [...slice].map((value) => value.toString(16).padStart(2, "0")).join(" ");
};

const findJpegMarkerOffset = (
  bytes: Uint8Array,
  markerHigh: number,
  markerLow: number,
  fromEnd: boolean = false,
): number | null => {
  if (fromEnd) {
    for (let index = bytes.byteLength - 2; index >= 0; index -= 1) {
      if (bytes[index] === markerHigh && bytes[index + 1] === markerLow) {
        return index;
      }
    }
    return null;
  }

  for (let index = 0; index < bytes.byteLength - 1; index += 1) {
    if (bytes[index] === markerHigh && bytes[index + 1] === markerLow) {
      return index;
    }
  }

  return null;
};

const describeBlobDiagnostics = async (blob: Blob): Promise<Record<string, unknown>> => {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  return {
    mimeType: blob.type,
    byteLength: bytes.byteLength,
    headHex: describeHexWindow(bytes, 16),
    tailHex: describeHexWindow(bytes, 16, true),
    startsWithJpegSoi:
      bytes.byteLength >= 2
        ? bytes[0] === 0xff && bytes[1] === 0xd8
        : false,
    firstSoiOffset: findJpegMarkerOffset(bytes, 0xff, 0xd8),
    lastEoiOffset: findJpegMarkerOffset(bytes, 0xff, 0xd9, true),
  };
};

const formatDiagnostics = (payload: Record<string, unknown>): string => {
  return JSON.stringify(payload);
};

const readImageBlobDimensions = async (blob: Blob): Promise<{ width: number; height: number }> => {
  if (typeof createImageBitmap === "function") {
    const bitmap = await createImageBitmap(blob);
    try {
      return {
        width: bitmap.width,
        height: bitmap.height,
      };
    } finally {
      bitmap.close();
    }
  }

  const decodedFrame = await decodeBlobToImageData(blob);
  return {
    width: decodedFrame.width,
    height: decodedFrame.height,
  };
};

const combineOrthogonalRotations = (
  first: OrthogonalRotation,
  second: OrthogonalRotation,
): OrthogonalRotation => {
  const normalized = (first + second) % 360;
  switch (normalized) {
    case 0:
    case 90:
    case 180:
    case 270:
      return normalized;
    default:
      throw new Error(`Unsupported orthogonal rotation combination: ${first} + ${second}.`);
  }
};

export const getCapturedDocumentProcessingSize = (
  width: number,
  height: number,
): { width: number; height: number } => {
  const scale = Math.min(
    1,
    CAPTURE_CV_MAX_WIDTH / Math.max(1, width),
    CAPTURE_CV_MAX_HEIGHT / Math.max(1, height),
  );
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
};

const getRotatedFrameDimensions = (
  width: number,
  height: number,
  rotation: OrthogonalRotation,
): { width: number; height: number } => {
  if (rotation === 90 || rotation === 270) {
    return {
      width: height,
      height: width,
    };
  }

  return {width, height};
};

const resolveStillFrameRotation = (
  previewDimensions: { width: number; height: number },
  stillDimensions: { width: number; height: number },
): {
  rotation: OrthogonalRotation;
  width: number;
  height: number;
  aspectDelta: number;
} => {
  let selectedRotation: OrthogonalRotation | null = null;
  let selectedDimensions: { width: number; height: number } | null = null;
  let selectedAspectDelta = Number.POSITIVE_INFINITY;
  let lastCompatibilityReason: string | null = null;

  for (const rotation of STILL_CAPTURE_ROTATION_CANDIDATES) {
    const candidateDimensions = getRotatedFrameDimensions(
      stillDimensions.width,
      stillDimensions.height,
      rotation,
    );
    const compatibility = evaluateFrameMappingCompatibility(
      previewDimensions,
      candidateDimensions,
    );

    if (!compatibility.compatible) {
      lastCompatibilityReason = compatibility.reason;
      continue;
    }

    if (selectedRotation === null || compatibility.aspectDelta < selectedAspectDelta) {
      selectedRotation = rotation;
      selectedDimensions = candidateDimensions;
      selectedAspectDelta = compatibility.aspectDelta;
    }
  }

  if (selectedRotation === null || !selectedDimensions) {
    throw new Error(
      `Preview/still mapping failed after trying all rotations. `
        + `preview=${previewDimensions.width}x${previewDimensions.height}, `
        + `still=${stillDimensions.width}x${stillDimensions.height}. `
        + (lastCompatibilityReason ? `Last attempt: ${lastCompatibilityReason}` : ``),
    );
  }

  return {
    rotation: selectedRotation,
    width: selectedDimensions.width,
    height: selectedDimensions.height,
    aspectDelta: selectedAspectDelta,
  };
};

const formatPerfMetric = (value: number | null): string => {
  return value === null ? "—" : value.toFixed(1);
};

export function useScannerCapture({
  dialogOpenRef,
  frameSourceRef,
  activeDetectionBackendRef,
  getLatestFrame,
  captureProcessingRef,
  orientationRef,
}: UseScannerCaptureParams): UseScannerCaptureResult {
  const processingRef = useRef(false);
  const processingCooldownRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestCvSnapshotRef = useRef<PreviewCaptureSnapshot | null>(null);
  const captureCommitGenerationRef = useRef(0);
  const captureCommitPendingRef = useRef(false);
  const autoCaptureRef = useRef(true);
  const capturedDocumentProcessVersionsRef = useRef<Map<string, number>>(new Map());
  const capturedDocumentQueueGenerationRef = useRef(0);
  const capturedDocumentQueueRef = useRef<Promise<void>>(Promise.resolve());

  const [autoCapture, setAutoCapture] = useState(true);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isCaptureCommitPending, setIsCaptureCommitPending] = useState(false);

  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});
  const imageEnhancement = useSettingsStore((state) => state.imageEnhancement);
  const scannerDetectionBackend = useSettingsStore((state) => state.scannerDetectionBackend);
  const scannerPostProcessBackend = useSettingsStore((state) => state.scannerPostProcessBackend);
  const scannerPipelineDebug = useSettingsStore((state) => state.scannerPipelineDebug);

  const capturedDocuments = useScannerStore((state) => state.capturedDocuments);
  const addCapturedDocument = useScannerStore((state) => state.addCapturedDocument);
  const updateCapturedDocument = useScannerStore((state) => state.updateCapturedDocument);
  const removeCapturedDocumentAction = useScannerStore((state) => state.removeCapturedDocument);
  const setCaptureDebug = useScannerStore((state) => state.setCaptureDebug);
  const setCvDebug = useScannerStore((state) => state.setCvDebug);

  const clearProcessingCooldown = useCallback(() => {
    if (processingCooldownRef.current) {
      clearTimeout(processingCooldownRef.current);
      processingCooldownRef.current = null;
    }
  }, []);

  const setProcessingState = useCallback((next: boolean) => {
    processingRef.current = next;
    setIsProcessing(next);
    if (captureProcessingRef) {
      captureProcessingRef.current = next;
    }
    setCvDebug({
      requestedBackend: scannerDetectionBackend,
      activeBackend: activeDetectionBackendRef.current,
      isProcessing: next,
      updatedAt: Date.now(),
    });
  }, [activeDetectionBackendRef, captureProcessingRef, scannerDetectionBackend, setCvDebug]);

  const setCaptureCommitPendingState = useCallback((next: boolean) => {
    captureCommitPendingRef.current = next;
    setIsCaptureCommitPending(next);
  }, []);

  const invalidatePendingCaptureCommits = useCallback(() => {
    captureCommitGenerationRef.current += 1;
    setCaptureCommitPendingState(false);
  }, [setCaptureCommitPendingState]);

  const canCommitCaptureResult = useCallback((captureGeneration: number): boolean => {
    return dialogOpenRef.current && captureGeneration === captureCommitGenerationRef.current;
  }, [dialogOpenRef]);

  const schedulePreviewCooldown = useCallback(() => {
    clearProcessingCooldown();
    processingCooldownRef.current = setTimeout(() => {
      setProcessingState(false);
      processingCooldownRef.current = null;
    }, PREVIEW_CAPTURE_COOLDOWN_MS);
  }, [clearProcessingCooldown, setProcessingState]);

  const publishCvDebug = useCallback((
    frame: ImageData | null,
    detectedPoints: Point[] | null,
    stable: boolean,
    pipeline: "idle" | "preview" | "single-hq",
    processing: boolean,
    processingDimensions?: { width: number; height: number },
  ) => {
    setCvDebug({
      requestedBackend: scannerDetectionBackend,
      activeBackend: activeDetectionBackendRef.current,
      pipeline,
      documentDetected: Boolean(detectedPoints && detectedPoints.length === 4),
      cornerCount: detectedPoints?.length ?? 0,
      cornerPoints: detectedPoints ?? [],
      isStable: stable,
      processingWidth: processingDimensions?.width ?? frame?.width ?? null,
      processingHeight: processingDimensions?.height ?? frame?.height ?? null,
      autoCaptureEnabled: autoCapture,
      isProcessing: processing,
      updatedAt: Date.now(),
    });
  }, [activeDetectionBackendRef, autoCapture, scannerDetectionBackend, setCvDebug]);

  const renderProcessedDocumentBlobFromSourceFile = useCallback(async (
    sourceFile: Blob,
    documentPoints: Point[] | null,
    outputRotation: OrthogonalRotation = 0,
    optionsOverride?: Partial<PostProcessOptions>,
  ): Promise<ProcessedDocumentRenderResult> => {
    const nativeResult = await processTauriScannerPostProcessSourceFile(sourceFile, {
      documentPoints,
      outputRotation,
      imageEnhancement: optionsOverride?.imageEnhancement ?? imageEnhancement,
      colorMode: optionsOverride?.colorMode ?? "auto",
      postprocessBackend: optionsOverride?.postprocessBackend ?? scannerPostProcessBackend,
      spineFlattening: optionsOverride?.spineFlattening ?? true,
      perspectiveTransform: optionsOverride?.perspectiveTransform ?? true,
      gridPostprocess: optionsOverride?.gridPostprocess ?? "none",
      pipelineDebug: scannerPipelineDebug,
    });
    const blob = new Blob([nativeResult.encodedBytes], {type: nativeResult.encodedMimeType});
    return {
      blob,
      decodeMs: nativeResult.decodeMs,
      refineMs: nativeResult.refineMs,
      inputWidth: nativeResult.inputWidth,
      inputHeight: nativeResult.inputHeight,
      outputWidth: nativeResult.outputWidth,
      outputHeight: nativeResult.outputHeight,
      perspectiveMs: nativeResult.perspectiveMs,
      flattenMs: nativeResult.flattenMs,
      enhanceMs: nativeResult.enhanceMs,
      modelMs: nativeResult.modelMs,
      residualWarpMs: nativeResult.residualWarpMs,
      rotateMs: nativeResult.rotateMs,
      encodeMs: nativeResult.encodeMs,
      postprocessBackend: nativeResult.postprocessBackend,
      modelId: nativeResult.modelId,
      controlGridShape: nativeResult.controlGridShape,
      effectiveDocumentPoints: nativeResult.effectiveDocumentPoints,
      refinementApplied: nativeResult.refinementApplied,
      localFlatteningApplied: nativeResult.localFlatteningApplied,
      residualWarpApplied: nativeResult.residualWarpApplied,
      residualWarpFallbackReason: nativeResult.residualWarpFallbackReason,
    };
  }, [
    imageEnhancement,
    scannerPostProcessBackend,
    scannerPipelineDebug,
  ]);

  const logHighQualityStillFailureDiagnostics = useCallback(async (
    stillCapture: ScannerStillCapture,
    error: unknown,
  ): Promise<void> => {
    try {
      const blobDiagnostics = await describeBlobDiagnostics(stillCapture.file);
      console.warn(
        `[Scanner][StillDiag] High-quality still decode failed. ${formatDiagnostics({
          error: error instanceof Error ? error.message : String(error),
          serial: stillCapture.serial,
          source: stillCapture.source,
          transport: stillCapture.transport,
          capturedAt: stillCapture.capturedAt,
          previewWidth: stillCapture.previewWidth,
          previewHeight: stillCapture.previewHeight,
          stillWidth: stillCapture.width,
          stillHeight: stillCapture.height,
          blob: blobDiagnostics,
        })}`,
      );
    } catch (diagnosticError) {
      console.warn("[Scanner][StillDiag] Failed to summarize high-quality still blob.", diagnosticError);
    }

    try {
      // Device log tailing is handled by the dedicated tauri_adb_start_log_tailer command.
      // The arbitrary shell command path (tauri_adb_shell) was removed for security hardening.
      console.warn("[Scanner][StillDiag] Device log tail unavailable — use the dedicated log tailer instead.");
    } catch (serverLogError) {
      console.warn("[Scanner][StillDiag] Failed to read device scanner server log tail.", serverLogError);
    }
  }, []);

  const buildHighQualityCaptureArtifact = useCallback(async (
    source: FrameSource,
    snapshot: PreviewCaptureSnapshot,
  ): Promise<PreparedCaptureArtifact> => {
    const totalStartedAt = performance.now();
    const stillCaptureStartedAt = performance.now();
    const stillCapture = await source.captureStillFrame();
    const stillCaptureMs = performance.now() - stillCaptureStartedAt;
    let stillDimensions: { width: number; height: number };
    let dimensionSource = "capture-metadata";
    try {
      if (
        typeof stillCapture.width === "number"
        && Number.isFinite(stillCapture.width)
        && stillCapture.width > 0
        && typeof stillCapture.height === "number"
        && Number.isFinite(stillCapture.height)
        && stillCapture.height > 0
      ) {
        stillDimensions = {
          width: stillCapture.width,
          height: stillCapture.height,
        };
      } else {
        stillDimensions = await readImageBlobDimensions(stillCapture.file);
        dimensionSource = "bitmap-fallback";
      }
    } catch (error) {
      await logHighQualityStillFailureDiagnostics(stillCapture, error);
      const fallbackReason = error instanceof Error ? error.message : String(error);
      throw new Error(`Failed to read high-quality still dimensions: ${fallbackReason}`);
    }
    const previewDimensions = {
      width: snapshot.frame.width,
      height: snapshot.frame.height,
    };
    const orientationStartedAt = performance.now();
    const selectedRotation = resolveStillFrameRotation(previewDimensions, {
      width: stillDimensions.width,
      height: stillDimensions.height,
    });
    const orientationMs = performance.now() - orientationStartedAt;

    const mappedPoints = snapshot.points && snapshot.points.length === 4
      ? mapPointsFromRotatedFrameToSource(
          scalePointsBetweenFrames(
            snapshot.points,
            previewDimensions,
            {
              width: selectedRotation.width,
              height: selectedRotation.height,
            },
          ),
          stillDimensions.width,
          stillDimensions.height,
          selectedRotation.rotation,
        )
      : null;
    const outputRotation = combineOrthogonalRotations(
      selectedRotation.rotation,
      orientationRef.current === "portrait" ? 90 : 0,
    );

    const totalMs = performance.now() - totalStartedAt;
    console.info(
      `[perf:still-prepare] ${stillCapture.transport} | total=${totalMs.toFixed(1)}ms`
      + ` capture=${stillCaptureMs.toFixed(1)}ms`
      + ` orient=${orientationMs.toFixed(1)}ms`
      + ` rotation=${selectedRotation.rotation}`
      + ` outputRotation=${outputRotation}`
      + ` aspectDelta=${selectedRotation.aspectDelta.toFixed(4)}`
      + ` sourceBlob=original-source-space`
      + ` dimensionSource=${dimensionSource}`
      + ` | preview=${previewDimensions.width}x${previewDimensions.height}`
      + ` still=${stillDimensions.width}x${stillDimensions.height}`
      + ` mapped=${selectedRotation.width}x${selectedRotation.height}`,
    );

    return {
      sourceBlob: stillCapture.file,
      sourceWidth: stillDimensions.width,
      sourceHeight: stillDimensions.height,
      points: mappedPoints,
      outputRotation,
      source: stillCapture.source,
    };
  }, [logHighQualityStillFailureDiagnostics, orientationRef]);

  const buildCaptureFile = useCallback((blob: Blob, outputNameBase: string): File => {
    const fileExtension = blob.type === "image/jpeg" ? "jpg" : "png";
    const fileType = blob.type || (fileExtension === "jpg" ? "image/jpeg" : "image/png");

    return new File([blob], `${outputNameBase}.${fileExtension}`, {
      type: fileType,
    });
  }, []);

  const invalidateCapturedDocumentQueue = useCallback(() => {
    capturedDocumentQueueGenerationRef.current += 1;
    capturedDocumentProcessVersionsRef.current.clear();
    capturedDocumentQueueRef.current = Promise.resolve();
  }, []);

  const queueCapturedDocumentProcessing = useCallback((
    documentId: string,
    sourceFile: File,
    outputNameBase: string,
    documentPoints: Point[] | null,
    outputRotation: OrthogonalRotation,
    options?: {
      redetectPoints?: boolean;
      overrides?: Partial<PostProcessOptions>;
    },
  ): void => {
    const queueGeneration = capturedDocumentQueueGenerationRef.current;
    const nextVersion = (capturedDocumentProcessVersionsRef.current.get(documentId) ?? 0) + 1;
    const shouldAttemptRedetect = Boolean(options?.redetectPoints);
    capturedDocumentProcessVersionsRef.current.set(documentId, nextVersion);

    updateCapturedDocument(documentId, {
      status: "processing",
      error: null,
      points: clonePoints(documentPoints),
      options: options?.overrides ? {
        imageEnhancement: options.overrides.imageEnhancement ?? imageEnhancement,
        colorMode: options.overrides.colorMode ?? "auto",
        postprocessBackend: options.overrides.postprocessBackend ?? scannerPostProcessBackend,
        spineFlattening: options.overrides.spineFlattening ?? true,
        perspectiveTransform: options.overrides.perspectiveTransform ?? true,
        gridPostprocess: options.overrides.gridPostprocess ?? "none",
      } : undefined,
    });
    setCaptureDebug({
      postProcessStatus: "processing",
      postProcessError: null,
      postProcessDecodeMs: null,
      postProcessRedetectMs: null,
      postProcessPerspectiveMs: null,
      postProcessEnhanceMs: null,
      postProcessModelMs: null,
      postProcessResidualWarpMs: null,
      postProcessEncodeMs: null,
      postProcessTotalMs: null,
      postProcessBackend: scannerPostProcessBackend,
      postProcessModelId: null,
      postProcessUsedRedetect: shouldAttemptRedetect,
      postProcessUsedPerspective: Boolean(documentPoints && documentPoints.length === 4),
      postProcessUsedEnhancement: imageEnhancement,
      postProcessUsedResidualWarp: false,
      postProcessFallbackReason: null,
      postProcessControlGridShape: null,
      postProcessInputWidth: null,
      postProcessInputHeight: null,
      postProcessOutputWidth: null,
      postProcessOutputHeight: null,
      postProcessUpdatedAt: Date.now(),
    });

    const runProcessing = async (): Promise<void> => {
      const totalStartedAt = performance.now();
      const decodeMs: number | null = null;
      let resolvedPoints = clonePoints(documentPoints);
      let redetectMs: number | null = null;

      if (shouldAttemptRedetect) {
        try {
          const redetectStartedAt = performance.now();
          const nativeResult = await detectDocumentWithTauriNativeOrt(sourceFile, {
            maxWidth: CAPTURE_CV_MAX_WIDTH,
            maxHeight: CAPTURE_CV_MAX_HEIGHT,
            backend: useSettingsStore.getState().scannerDetectionBackend,
          });
          redetectMs = performance.now() - redetectStartedAt;
          const detectedPoints = nativeResult.points ?? null;
          if (detectedPoints && detectedPoints.length === 4) {
            resolvedPoints = detectedPoints;
          }
        } catch (error) {
          console.warn("[Scanner] Failed to redetect captured document corners:", error);
        }
      }

      const processedDocument = await renderProcessedDocumentBlobFromSourceFile(
        sourceFile,
        resolvedPoints,
        outputRotation,
        options?.overrides,
      );
      const effectiveDecodeMs = decodeMs ?? processedDocument.decodeMs;
      const inputWidth = processedDocument.inputWidth;
      const inputHeight = processedDocument.inputHeight;
      const processedFile = buildCaptureFile(processedDocument.blob, outputNameBase);
      const effectivePoints = clonePoints(processedDocument.effectiveDocumentPoints ?? resolvedPoints);

      if (
        queueGeneration !== capturedDocumentQueueGenerationRef.current
        || capturedDocumentProcessVersionsRef.current.get(documentId) !== nextVersion
      ) {
        return;
      }

      updateCapturedDocument(documentId, {
        file: processedFile,
        points: effectivePoints,
        status: "ready",
        error: null,
        documentDetected: Boolean(effectivePoints && effectivePoints.length === 4),
      });
      const totalMs = performance.now() - totalStartedAt;
      setCaptureDebug({
        postProcessStatus: "success",
        postProcessError: null,
        postProcessDecodeMs: effectiveDecodeMs,
        postProcessRedetectMs: redetectMs,
        postProcessPerspectiveMs: processedDocument.perspectiveMs,
        postProcessEnhanceMs: processedDocument.enhanceMs,
        postProcessModelMs: processedDocument.modelMs,
        postProcessResidualWarpMs: processedDocument.residualWarpMs,
        postProcessEncodeMs: processedDocument.encodeMs,
        postProcessTotalMs: totalMs,
        postProcessBackend: processedDocument.postprocessBackend,
        postProcessModelId: processedDocument.modelId,
        postProcessUsedRedetect: shouldAttemptRedetect,
        postProcessUsedPerspective: Boolean(effectivePoints && effectivePoints.length === 4),
        postProcessUsedEnhancement: imageEnhancement,
        postProcessUsedResidualWarp: processedDocument.residualWarpApplied,
        postProcessFallbackReason: processedDocument.residualWarpFallbackReason,
        postProcessControlGridShape: processedDocument.controlGridShape,
        postProcessInputWidth: inputWidth,
        postProcessInputHeight: inputHeight,
        postProcessOutputWidth: processedDocument.outputWidth,
        postProcessOutputHeight: processedDocument.outputHeight,
        postProcessUpdatedAt: Date.now(),
      });
      console.info(
        `[perf:postprocess] ${outputNameBase} | total=${totalMs.toFixed(1)}ms`
        + ` decode=${formatPerfMetric(effectiveDecodeMs)}ms`
        + ` refine=${formatPerfMetric(processedDocument.refineMs)}ms`
        + ` redetect=${formatPerfMetric(redetectMs)}ms`
        + ` perspective=${formatPerfMetric(processedDocument.perspectiveMs)}ms`
        + ` model=${formatPerfMetric(processedDocument.modelMs)}ms`
        + ` residualWarp=${formatPerfMetric(processedDocument.residualWarpMs)}ms`
        + ` flatten=${formatPerfMetric(processedDocument.flattenMs)}ms`
        + ` enhance=${formatPerfMetric(processedDocument.enhanceMs)}ms`
        + ` rotate=${formatPerfMetric(processedDocument.rotateMs)}ms`
        + ` encode=${processedDocument.encodeMs.toFixed(1)}ms`
        + ` outputRotation=${outputRotation}`
        + ` backend=${processedDocument.postprocessBackend}`
        + ` modelId=${processedDocument.modelId ?? "—"}`
        + ` refinementApplied=${processedDocument.refinementApplied}`
        + ` residualWarpApplied=${processedDocument.residualWarpApplied}`
        + ` localFlatteningApplied=${processedDocument.localFlatteningApplied}`
        + ` residualWarpFallback=${processedDocument.residualWarpFallbackReason ?? "—"}`
        + ` | ${inputWidth}x${inputHeight}`
        + ` -> ${processedDocument.outputWidth}x${processedDocument.outputHeight}`,
      );
    };

    capturedDocumentQueueRef.current = capturedDocumentQueueRef.current
      .catch(() => undefined)
      .then(runProcessing)
      .catch((error) => {
        if (
          queueGeneration !== capturedDocumentQueueGenerationRef.current
          || capturedDocumentProcessVersionsRef.current.get(documentId) !== nextVersion
        ) {
          return;
        }

        const message = error instanceof Error ? error.message : String(error);
        updateCapturedDocument(documentId, {
          points: clonePoints(documentPoints),
          status: "failed",
          error: message,
        });
        setCaptureDebug({
          postProcessStatus: "error",
          postProcessError: message,
          postProcessBackend: scannerPostProcessBackend,
          postProcessUpdatedAt: Date.now(),
        });
        toast.error(t("toasts.post-process-failed", {message}));
      });
  }, [
    buildCaptureFile,
    imageEnhancement,
    renderProcessedDocumentBlobFromSourceFile,
    setCaptureDebug,
    scannerPostProcessBackend,
    t,
    updateCapturedDocument,
  ]);

  const saveCapturedDocument = useCallback((
    artifact: PreparedCaptureArtifact,
    documentDetected: boolean,
    captureSource: "preview-stream" | "single-hq",
  ) => {
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    const prefix = captureSource === "single-hq" ? "scan_hq" : "scan_preview";
    const outputNameBase = `${prefix}_${timestamp}`;
    const sourceExtension = artifact.sourceBlob.type === "image/jpeg" ? "jpg" : "png";
    const sourceType = artifact.sourceBlob.type || (sourceExtension === "jpg" ? "image/jpeg" : "image/png");
    const sourceFile = new File([artifact.sourceBlob], `${outputNameBase}_source.${sourceExtension}`, {
      type: sourceType,
    });
    const pointsForDocument = clonePoints(artifact.points);
    const initialQuadAssessment = assessDocumentQuad(
      pointsForDocument,
      artifact.sourceWidth,
      artifact.sourceHeight,
    );
    const needsPostProcessing = Boolean(
      artifact.outputRotation !== 0
      || imageEnhancement
      || (pointsForDocument && pointsForDocument.length === 4)
    );
    const documentId = crypto.randomUUID();

    addCapturedDocument({
      id: documentId,
      file: sourceFile,
      sourceFile,
      points: pointsForDocument,
      status: needsPostProcessing ? "processing" : "ready",
      error: null,
      documentDetected,
      captureSource,
      sourceWidth: artifact.sourceWidth,
      sourceHeight: artifact.sourceHeight,
      outputNameBase,
      outputRotation: artifact.outputRotation,
    });

    if (needsPostProcessing) {
      if (!initialQuadAssessment.trustworthy) {
        console.warn(
          "[Scanner] Captured document points are not trustworthy; forcing source-image redetect before post-process.",
          {
            captureSource,
            reason: initialQuadAssessment.reason,
            points: pointsForDocument,
            sourceWidth: artifact.sourceWidth,
            sourceHeight: artifact.sourceHeight,
          },
        );
      }
      queueCapturedDocumentProcessing(
        documentId,
        sourceFile,
        outputNameBase,
        pointsForDocument,
        artifact.outputRotation,
        {
          redetectPoints: !initialQuadAssessment.trustworthy,
        },
      );
    }

    setCaptureDebug({
      lastCaptureSource: captureSource,
      lastCaptureWidth: artifact.sourceWidth,
      lastCaptureHeight: artifact.sourceHeight,
      lastCaptureAt: Date.now(),
      lastCaptureError: null,
      lastCaptureDocumentDetected: documentDetected,
    });
  }, [addCapturedDocument, imageEnhancement, queueCapturedDocumentProcessing, setCaptureDebug]);

  const createCaptureSnapshot = useCallback((
    frame: ImageData,
    sourcePoints: Point[] | null,
  ): PreviewCaptureSnapshot => {
    return {
      frame: cloneFrame(frame),
      points: clonePoints(sourcePoints),
    };
  }, []);

  const captureDocument = useCallback(async (
    snapshot: PreviewCaptureSnapshot,
    trigger: "manual" | "auto",
  ) => {
    if (processingRef.current || !dialogOpenRef.current) {
      return;
    }

    const captureGeneration = captureCommitGenerationRef.current;
    const {frame, points: sourcePoints} = snapshot;
    const highQualityCapabilities = frameSourceRef.current?.getState().capabilities;
    const highQualityEnabled = Boolean(highQualityCapabilities?.highQualityStillCapture);

    setProcessingState(true);
    setCaptureCommitPendingState(true);
    publishCvDebug(
      frame,
      sourcePoints,
      Boolean(sourcePoints && sourcePoints.length === 4),
      highQualityEnabled ? "single-hq" : "preview",
      true,
    );
    setCaptureDebug({
      highQualityStatus: highQualityEnabled ? "capturing" : "idle",
      highQualitySource: null,
      highQualityFallbackReason: null,
      lastCaptureError: null,
    });
    toast.info(
      trigger === "auto"
        ? t("toasts.auto-capturing-preview")
        : t("toasts.capturing-preview"),
    );

    try {
      const highQualitySource = highQualityEnabled ? frameSourceRef.current : null;
      let captureArtifact: PreparedCaptureArtifact | null = null;
      let captureSource: "preview-stream" | "single-hq" = "preview-stream";
      const requiresHighQualitySource = highQualityEnabled;

      if (highQualitySource) {
        const highQualityAttempts = requiresHighQualitySource ? 2 : 1;
        let lastHighQualityError: unknown = null;

        for (let attempt = 1; attempt <= highQualityAttempts; attempt += 1) {
          try {
            setCaptureDebug({
              highQualityStatus: "processing",
              highQualitySource: null,
              highQualityFallbackReason: null,
            });
            const artifact = await buildHighQualityCaptureArtifact(highQualitySource, snapshot);
            captureArtifact = artifact;
            captureSource = "single-hq";
            setCaptureDebug({
              highQualityStatus: "success",
              highQualitySource: artifact.source ?? null,
              highQualityFallbackReason: null,
            });
            lastHighQualityError = null;
            break;
          } catch (highQualityError) {
            lastHighQualityError = highQualityError;
            if (attempt < highQualityAttempts) {
              console.warn(`[Scanner] High-quality still capture attempt ${attempt} failed, retrying:`, highQualityError);
            }
          }
        }

        if (lastHighQualityError) {
          const fallbackReason = lastHighQualityError instanceof Error
            ? lastHighQualityError.message
            : String(lastHighQualityError);
          const highQualityStillStillAvailable = Boolean(
            frameSourceRef.current?.getState().capabilities.highQualityStillCapture,
          );
          console.warn("[Scanner] High-quality still capture failed:", lastHighQualityError);
          setCaptureDebug({
            highQualityStatus: "error",
            highQualitySource: null,
            highQualityFallbackReason: fallbackReason,
          });

          if (requiresHighQualitySource && highQualityStillStillAvailable) {
            throw new Error(`Scanner capture requires a usable high-quality still source: ${fallbackReason}`);
          }
        }
      }

      if (!captureArtifact) {
        throw new Error("Scanner capture requires a high-quality still source but none was available.");
      }

      if (!canCommitCaptureResult(captureGeneration)) {
        return;
      }
      saveCapturedDocument(
        captureArtifact,
        Boolean(sourcePoints && sourcePoints.length === 4),
        captureSource,
      );
      if (!canCommitCaptureResult(captureGeneration)) {
        setProcessingState(false);
        return;
      }
      toast.success(t("toasts.preview-ready"));
      schedulePreviewCooldown();
    } catch (error) {
      if (!canCommitCaptureResult(captureGeneration)) {
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      console.error("[Scanner] Preview capture failed:", error);
      setCaptureDebug({lastCaptureError: message});
      setProcessingState(false);
      toast.error(t("toasts.preview-failed", {message}));
    } finally {
      if (captureGeneration === captureCommitGenerationRef.current) {
        setCaptureCommitPendingState(false);
      }
    }
  }, [
    buildHighQualityCaptureArtifact,
    canCommitCaptureResult,
    dialogOpenRef,
    frameSourceRef,
    publishCvDebug,
    saveCapturedDocument,
    schedulePreviewCooldown,
    setCaptureCommitPendingState,
    setCaptureDebug,
    setProcessingState,
    t,
  ]);

  const capture = useCallback(async (): Promise<void> => {
    const cvSnapshot = latestCvSnapshotRef.current;
    if (cvSnapshot) {
      await captureDocument(createCaptureSnapshot(
        cvSnapshot.frame,
        cvSnapshot.points,
      ), "manual");
      return;
    }

    const frame = getLatestFrame();
    if (!frame) {
      return;
    }

    await captureDocument(createCaptureSnapshot(frame, null), "manual");
  }, [captureDocument, createCaptureSnapshot, getLatestFrame]);

  const requestAutoCapture = useCallback((frame: ImageData, points: Point[] | null) => {
    const hasTrustworthyQuad = isDocumentQuadTrustworthy(points, frame.width, frame.height);
    const snapshot = createCaptureSnapshot(frame, hasTrustworthyQuad ? points : null);
    latestCvSnapshotRef.current = snapshot;

    if (
      !autoCaptureRef.current
      || processingRef.current
      || processingCooldownRef.current
      || !hasTrustworthyQuad
    ) {
      return;
    }

    void captureDocument(snapshot, "auto");
  }, [captureDocument, createCaptureSnapshot]);

  const removeCapturedDocument = useCallback((documentId: string) => {
    capturedDocumentProcessVersionsRef.current.delete(documentId);
    removeCapturedDocumentAction(documentId);
  }, [removeCapturedDocumentAction]);

  const reprocessCapturedDocument = useCallback((
    documentId: string,
    nextPoints: Point[],
    _options?: PostProcessOptions,
  ) => {
    const document = capturedDocuments.find((entry) => entry.id === documentId);
    if (!document) {
      return;
    }

    queueCapturedDocumentProcessing(
      documentId,
      document.sourceFile,
      document.outputNameBase,
      nextPoints,
      document.outputRotation,
      {
        redetectPoints: false,
        overrides: _options,
      },
    );
  }, [capturedDocuments, queueCapturedDocumentProcessing]);

  const getEditorPreview = useCallback(async (
    doc: ScannerCapturedDocument,
    points: Point[],
    _ppOptions: PostProcessOptions,
  ): Promise<{ blob: Blob; processingMs: number }> => {
    const startedAt = performance.now();
    const result = await renderProcessedDocumentBlobFromSourceFile(
      doc.sourceFile,
      points,
      doc.outputRotation,
      _ppOptions,
    );
    const processingMs = performance.now() - startedAt;
    return {blob: result.blob, processingMs};
  }, [renderProcessedDocumentBlobFromSourceFile]);

  const resetCaptureRuntime = useCallback(() => {
    invalidatePendingCaptureCommits();
    invalidateCapturedDocumentQueue();
    clearProcessingCooldown();
    setProcessingState(false);
    if (captureProcessingRef) {
      captureProcessingRef.current = false;
    }
    setCaptureCommitPendingState(false);
    latestCvSnapshotRef.current = null;
  }, [
    captureProcessingRef,
    clearProcessingCooldown,
    invalidateCapturedDocumentQueue,
    invalidatePendingCaptureCommits,
    setCaptureCommitPendingState,
    setProcessingState,
  ]);

  const resetCaptureDebug = useCallback(() => {
    publishCvDebug(null, null, false, "idle", false);
    setCaptureDebug({
      highQualityStatus: "idle",
      highQualitySource: null,
      highQualityFallbackReason: null,
      lastCaptureError: null,
      lastCaptureSource: null,
      lastCaptureAt: null,
      lastCaptureWidth: null,
      lastCaptureHeight: null,
      lastCaptureDocumentDetected: false,
      postProcessStatus: "idle",
      postProcessError: null,
      postProcessDecodeMs: null,
      postProcessRedetectMs: null,
      postProcessPerspectiveMs: null,
      postProcessEnhanceMs: null,
      postProcessModelMs: null,
      postProcessResidualWarpMs: null,
      postProcessEncodeMs: null,
      postProcessTotalMs: null,
      postProcessFlattenMs: null,
      postProcessBackend: "heuristic",
      postProcessModelId: null,
      postProcessUsedRedetect: false,
      postProcessUsedPerspective: false,
      postProcessUsedEnhancement: false,
      postProcessUsedResidualWarp: false,
      postProcessFallbackReason: null,
      postProcessControlGridShape: null,
      postProcessInputWidth: null,
      postProcessInputHeight: null,
      postProcessOutputWidth: null,
      postProcessOutputHeight: null,
      postProcessUpdatedAt: null,
    });
  }, [publishCvDebug, setCaptureDebug]);

  useEffect(() => {
    autoCaptureRef.current = autoCapture;
    setCvDebug({
      requestedBackend: scannerDetectionBackend,
      activeBackend: activeDetectionBackendRef.current,
      autoCaptureEnabled: autoCapture,
      isProcessing,
      updatedAt: Date.now(),
    });
  }, [activeDetectionBackendRef, autoCapture, isProcessing, scannerDetectionBackend, setCvDebug]);

  useEffect(() => clearProcessingCooldown, [clearProcessingCooldown]);

  return {
    capture,
    requestAutoCapture,
    autoCapture,
    setAutoCapture,
    isCapturing: isProcessing,
    isCaptureCommitPending,
    capturedDocuments,
    removeCapturedDocument,
    reprocessCapturedDocument,
    getEditorPreview,
    invalidatePendingCommits: invalidatePendingCaptureCommits,
    invalidateQueue: invalidateCapturedDocumentQueue,
    resetCaptureRuntime,
    resetCaptureDebug,
  };
}