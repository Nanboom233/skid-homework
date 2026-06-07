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
} from "../lib/scanner";
import {
  decodeBlobToImageData,
  type OrthogonalRotation,
} from "../lib/scanner/image-data";
import {mapPointsFromRotatedFrameToSource, type PreviewOrientation} from "../lib/scanner/preview-orientation";
import {isDocumentQuadTrustworthy} from "../lib/scanner/document-quad";
import {
  type ScannerCapturedDocument,
  useScannerStore,
} from "../store/scanner-store";
import {
  type ScannerDetectionBackend,
  useSettingsStore,
} from "@/store/settings-store";

const PREVIEW_CAPTURE_COOLDOWN_MS = 1200;
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

  const [autoCapture, setAutoCapture] = useState(true);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isCaptureCommitPending, setIsCaptureCommitPending] = useState(false);

  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});
  const scannerDetectionBackend = useSettingsStore((state) => state.scannerDetectionBackend);

  const capturedDocuments = useScannerStore((state) => state.capturedDocuments);
  const addCapturedDocument = useScannerStore((state) => state.addCapturedDocument);
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

  const saveCapturedDocument = useCallback((
    artifact: PreparedCaptureArtifact,
    documentDetected: boolean,
    captureSource: "preview-stream" | "single-hq",
  ) => {
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    const prefix = captureSource === "single-hq" ? "scan_hq" : "scan_preview";
    const outputNameBase = `${prefix}_${timestamp}`;
    const sourceFile = buildCaptureFile(artifact.sourceBlob, outputNameBase);
    const pointsForDocument = clonePoints(artifact.points);
    const documentId = crypto.randomUUID();

    addCapturedDocument({
      id: documentId,
      file: sourceFile,
      sourceFile,
      points: pointsForDocument,
      status: "ready",
      error: null,
      documentDetected,
      captureSource,
      sourceWidth: artifact.sourceWidth,
      sourceHeight: artifact.sourceHeight,
      outputNameBase,
      outputRotation: artifact.outputRotation,
    });

    setCaptureDebug({
      lastCaptureSource: captureSource,
      lastCaptureWidth: artifact.sourceWidth,
      lastCaptureHeight: artifact.sourceHeight,
      lastCaptureAt: Date.now(),
      lastCaptureError: null,
      lastCaptureDocumentDetected: documentDetected,
    });
  }, [addCapturedDocument, buildCaptureFile, setCaptureDebug]);

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
    removeCapturedDocumentAction(documentId);
  }, [removeCapturedDocumentAction]);

  const resetCaptureRuntime = useCallback(() => {
    invalidatePendingCaptureCommits();
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
    resetCaptureRuntime,
    resetCaptureDebug,
  };
}
