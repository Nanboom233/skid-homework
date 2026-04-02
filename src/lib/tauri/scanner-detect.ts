import type {Point} from "@/lib/scanner/document-detector";

import {isTauri} from "./platform";

export interface TauriScannerYoloResourceStatus {
  key: string;
  relativePath: string;
  resolvedPath: string | null;
  exists: boolean;
  required: boolean;
}

export interface TauriScannerYoloModelConfig {
  id: string;
  kind: string;
  task: string;
  modelPath: string;
  inputName?: string | null;
  outputName?: string | null;
  inputSize?: [number, number] | null;
}

export interface TauriScannerYoloWindowsConfig {
  preferredProvider: string;
  runtimeLibrary: string;
  sharedLibrary: string;
  providerLibrary: string;
}

export interface TauriScannerYoloLinuxConfig {
  preferredProviders: string[];
  runtimeLibrary: string;
  providerLibraries: string[];
  officialGpuReleaseArtifact: string;
}

export interface TauriScannerYoloConfig {
  stage: string;
  task: string;
  intendedPrimaryModel: TauriScannerYoloModelConfig;
  activePublicBaseline: TauriScannerYoloModelConfig;
  windows?: TauriScannerYoloWindowsConfig | null;
  linux?: TauriScannerYoloLinuxConfig | null;
  notes: string[];
}

export interface TauriScannerYoloConfigResponse {
  config: TauriScannerYoloConfig;
  source: string;
  resolvedPath: string;
  writablePath: string;
}

export interface TauriScannerYoloProbeResult {
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

export const readTauriScannerYoloConfig = async (): Promise<TauriScannerYoloConfigResponse> => {
  if (!isTauri()) {
    throw new Error("Scanner YOLO config is only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
  return await invoke<TauriScannerYoloConfigResponse>("tauri_scanner_read_yolo_config");
};

export const writeTauriScannerYoloConfig = async (
  config: TauriScannerYoloConfig,
): Promise<TauriScannerYoloConfigResponse> => {
  if (!isTauri()) {
    throw new Error("Scanner YOLO config is only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
  return await invoke<TauriScannerYoloConfigResponse>("tauri_scanner_write_yolo_config", {
    config,
  });
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

export const detectDocumentWithTauriNativeYoloRgba = async (
  frame: ImageData,
  options?: {
    maxWidth?: number;
    maxHeight?: number;
  },
): Promise<TauriScannerNativeYoloDetectResult> => {
  if (!isTauri()) {
    throw new Error("Native YOLO detection is only available in Tauri desktop builds.");
  }

  const rgbaBytes = new Uint8Array(
    frame.data.buffer,
    frame.data.byteOffset,
    frame.data.byteLength,
  );
  const {invoke} = await import("@tauri-apps/api/core");

  return await invoke<TauriScannerNativeYoloDetectResult>(
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
