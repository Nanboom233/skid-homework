import type {Point} from "@/lib/scanner/document-detector";

import {isTauri} from "./platform";

export interface TauriScannerYoloResourceStatus {
  key: string;
  relativePath: string;
  resolvedPath: string | null;
  exists: boolean;
  required: boolean;
}

export interface TauriScannerYoloProbeResult {
  stage: string;
  platform: string;
  platformTarget: string;
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
  resources: TauriScannerYoloResourceStatus[];
  message: string;
}

export interface TauriScannerNativeYoloDetectResult {
  stage: string;
  processingMs: number;
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

export const probeTauriScannerYolo = async (): Promise<TauriScannerYoloProbeResult> => {
  if (!isTauri()) {
    throw new Error("Native YOLO probe is only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
  return await invoke<TauriScannerYoloProbeResult>("tauri_scanner_probe_yolo");
};

export const detectDocumentWithTauriNativeYolo = async (
  source: Blob | ArrayBuffer | Uint8Array,
  options?: {
    maxWidth?: number;
    maxHeight?: number;
  },
): Promise<TauriScannerNativeYoloDetectResult> => {
  if (!isTauri()) {
    throw new Error("Native YOLO detection is only available in Tauri desktop builds.");
  }

  const sourceBytes = await normalizeSourceBytes(source);
  const {invoke} = await import("@tauri-apps/api/core");

  return await invoke<TauriScannerNativeYoloDetectResult>(
    "tauri_scanner_detect_document",
    {
      request: {
        sourceBytes,
        maxWidth: options?.maxWidth,
        maxHeight: options?.maxHeight,
      },
    },
  );
};
