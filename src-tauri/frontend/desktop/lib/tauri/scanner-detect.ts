import type {Point} from "../scanner/types";

import {isTauri} from "./platform";

export interface TauriScannerDetectResourceStatus {
  key: string;
  relativePath: string;
  resolvedPath: string | null;
  exists: boolean;
  required: boolean;
}

export interface TauriScannerDetectProbeResult {
  stage: string;
  platform: string;
  platformTarget: string;
  configSource: string;
  configPath: string | null;
  preferredProvider: string;
  providerCandidates: string[];
  selectedModelId: string | null;
  selectedModelKind: string | null;
  selectedModelTask: string | null;
  selectedModelPath: string | null;
  runtimeReady: boolean;
  preferredProviderReady: boolean;
  modelReady: boolean;
  sessionReady: boolean;
  detectionImplemented: boolean;
  ortBuildInfo: string | null;
  runtimeError: string | null;
  sessionError: string | null;
  resourceResolutionSource: string;
  resourceBaseDir: string | null;
  resources: TauriScannerDetectResourceStatus[];
  message: string;
}

export interface TauriScannerNativeOrtDetectResult {
  stage: string;
  processingMs: number;
  inputTransport: string;
  inputWidth: number | null;
  inputHeight: number | null;
  selectedModelId: string | null;
  selectedModelKind: string | null;
  selectedModelTask: string | null;
  runtimeReady: boolean;
  preferredProvider: string;
  preferredProviderReady: boolean;
  modelReady: boolean;
  sessionReady: boolean;
  detectionImplemented: boolean;
  ortBuildInfo: string | null;
  runtimeError: string | null;
  sessionError: string | null;
  points: Point[] | null;
  message: string;
}

const normalizeSourceBytes = async (
  source: Blob | ArrayBuffer | Uint8Array,
): Promise<Uint8Array> => {
  if (source instanceof Blob) {
    return new Uint8Array(await source.arrayBuffer());
  }

  if (source instanceof ArrayBuffer) {
    return new Uint8Array(source);
  }

  return source;
};

export const probeTauriScannerDetect = async (): Promise<TauriScannerDetectProbeResult> => {
  if (!isTauri()) {
    throw new Error("Native ORT probe is only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
  return await invoke<TauriScannerDetectProbeResult>("tauri_scanner_probe_detect");
};

export const detectDocumentWithTauriNativeOrt = async (
  source: Blob | ArrayBuffer | Uint8Array,
  options?: {
    maxWidth?: number;
    maxHeight?: number;
    backend?: string;
  },
): Promise<TauriScannerNativeOrtDetectResult> => {
  if (!isTauri()) {
    throw new Error("Native ORT detection is only available in Tauri desktop builds.");
  }

  const sourceBytes = await normalizeSourceBytes(source);
  const {invoke} = await import("@tauri-apps/api/core");

  return await invoke<TauriScannerNativeOrtDetectResult>(
    "tauri_scanner_detect_document",
    {
      sourceBytes,
      request: {
        maxWidth: options?.maxWidth,
        maxHeight: options?.maxHeight,
        backend: options?.backend,
      },
    },
  );
};

export const detectDocumentWithTauriNativeOrtRgba = async (
  frame: ImageData,
  options?: {
    maxWidth?: number;
    maxHeight?: number;
  },
): Promise<TauriScannerNativeOrtDetectResult> => {
  if (!isTauri()) {
    throw new Error("Native ORT detection is only available in Tauri desktop builds.");
  }

  const rgbaBytes = new Uint8Array(
    frame.data.buffer,
    frame.data.byteOffset,
    frame.data.byteLength,
  );
  const {invoke} = await import("@tauri-apps/api/core");

  return await invoke<TauriScannerNativeOrtDetectResult>(
    "tauri_scanner_detect_document",
    {
      request: {
        rgbaBytes,
        rgbaWidth: frame.width,
        rgbaHeight: frame.height,
        maxWidth: options?.maxWidth,
        maxHeight: options?.maxHeight,
      },
    },
  );
};


// ---------------------------------------------------------------------------
// Detection Loop (Rust-driven background detection)
// ---------------------------------------------------------------------------

export interface DetectionLoopConfig {
  backend: string;
  intervalMs?: number;
  stableFrames?: number;
  varianceThreshold?: number;
  stableHoldMs?: number;
  missGraceFrames?: number;
  missGraceMs?: number;
  smoothingThresholdPx?: number;
  smoothingFactor?: number;
}

export interface DetectionResultEvent {
  points: Point[] | null;
  effectivePoints: Point[] | null;
  isStable: boolean;
  autoCaptureTriggered: boolean;
  detectionMs: number;
  backend: string;
  frameWidth: number;
  frameHeight: number;
  message: string;
}

export const startTauriDetectionLoop = async (config: DetectionLoopConfig): Promise<void> => {
  if (!isTauri()) {
    throw new Error("Detection loop is only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
  await invoke("tauri_scanner_start_detection_loop", {config});
};

export const stopTauriDetectionLoop = async (): Promise<void> => {
  if (!isTauri()) {
    return;
  }

  const {invoke} = await import("@tauri-apps/api/core");
  try {
    await invoke("tauri_scanner_stop_detection_loop");
  } catch {
    // Silently ignore "no loop running" errors on stop
  }
};

/**
 * Subscribe to detection result events from the Rust detection loop.
 * Auto-capture is derived from DetectionResultEvent.autoCaptureTriggered,
 * so this helper treats scanner-detection as the single event source and
 * does not subscribe to the legacy scanner-auto-capture channel.
 * Returns an unsubscribe function.
 */
export const listenTauriDetectionEvents = async (
  onDetection: (event: DetectionResultEvent) => void,
): Promise<() => void> => {
  if (!isTauri()) {
    return () => {};
  }

  const {listen} = await import("@tauri-apps/api/event");
  const detectionUnsub = await listen<DetectionResultEvent>("scanner-detection", (event) => {
    onDetection(event.payload);
  });

  return () => {
    detectionUnsub();
  };
};
