import type {Point} from "../scanner/types";
import type {OrthogonalRotation} from "../scanner/image-data";
import type {ScannerPostProcessBackend} from "@/store/settings-store";

import {isTauri} from "./platform";

type TauriRawChannelPayload = string | ArrayBuffer | Uint8Array | number[];

const NATIVE_POST_PROCESS_TIMEOUT_MS = 120_000;
const NATIVE_POST_PROCESS_PARTIAL_TIMEOUT_MS = 1_500;

export interface TauriScannerPostProcessResult {
  processingMs: number;
  decodeMs: number;
  refineMs: number | null;
  perspectiveMs: number | null;
  flattenMs: number | null;
  enhanceMs: number | null;
  modelMs: number | null;
  residualWarpMs: number | null;
  rotateMs: number | null;
  encodeMs: number;
  inputWidth: number;
  inputHeight: number;
  outputWidth: number;
  outputHeight: number;
  encodedMimeType: string;
  postprocessBackend: ScannerPostProcessBackend;
  modelId: string | null;
  controlGridShape: string | null;
  effectiveDocumentPoints: Point[] | null;
  refinementApplied: boolean;
  localFlatteningApplied: boolean;
  residualWarpApplied: boolean;
  residualWarpFallbackReason: string | null;
  encodedBytes: ArrayBuffer;
}

export interface ScannerAssetsError {
  code: string;
  retryable: boolean;
  details?: string;
}

export interface ScannerAssetsManifestSummary {
  schemaVersion: number;
  assetVersion: string;
  platformTarget: string;
}

export interface ScannerAssetsStatus {
  state: string;
  platformTarget: string;
  expectedAssetTag: string;
  defaultAssetUrl: string;
  currentDir?: string;
  manifest?: ScannerAssetsManifestSummary;
  lastError?: ScannerAssetsError;
}

export interface ScannerAssetsProgress {
  phase: "fetching" | "unpacking" | "verifying" | "activating" | "completed" | "failed";
  bytesDone?: number;
  bytesTotal?: number;
  error?: ScannerAssetsError;
}

export interface ScannerAssetsInstallResult {
  installed: boolean;
  assetVersion: string;
  platformTarget: string;
  currentDir: string;
}

export interface ScannerAssetsUpdateCheck {
  platformTarget: string;
  currentAssetVersion?: string;
  targetAssetTag: string;
  updateAvailable: boolean;
}

export interface ScannerOrtOutletStatus {
  name: string;
  dtype: string;
  shape?: string;
}

export interface ScannerOrtModelStatus {
  id: string;
  role: string;
  relativePath: string;
  resolvedPath?: string;
  fileExists: boolean;
  sessionReady: boolean;
  sessionError?: string;
  inputs: ScannerOrtOutletStatus[];
  outputs: ScannerOrtOutletStatus[];
}

export interface ScannerOrtResourceStatus {
  relativePath: string;
  resolvedPath?: string;
  exists: boolean;
  required: boolean;
}

export interface ScannerOrtProbeStatus {
  stage: string;
  platform: string;
  platformTarget: string;
  resourceResolutionSource: string;
  resourceBaseDir?: string;
  selectedRuntimeLibraryPath?: string;
  loadedRuntimeLibraryPath?: string;
  runtimeLibraryPath?: string;
  runtimePathMismatch: boolean;
  runtimeReady: boolean;
  modelLoadReady: boolean;
  preferredProvider: string;
  preferredProviderReady: boolean;
  providerCandidates: string[];
  availableProviders: string[];
  ortBuildInfo?: string;
  runtimeError?: string;
  resources: ScannerOrtResourceStatus[];
  models: ScannerOrtModelStatus[];
  message: string;
}

const invokeTauriCommand = async <T>(
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> => {
  if (!isTauri()) {
    throw new Error("Scanner commands are only available in Tauri desktop builds.");
  }
  const {invoke} = await import("@tauri-apps/api/core");
  return await invoke<T>(command, payload);
};

const decodeBase64ToUint8Array = (base64: string): Uint8Array => {
  const normalized = base64.replace(/\s+/g, "");
  const binary = atob(normalized);
  const bytes = new Uint8Array(binary.length);

  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }

  return bytes;
};

const normalizeTauriRawChannelPayload = (payload: TauriRawChannelPayload): Uint8Array => {
  if (typeof payload === "string") {
    return decodeBase64ToUint8Array(payload);
  }

  if (payload instanceof ArrayBuffer) {
    return new Uint8Array(payload);
  }

  if (payload instanceof Uint8Array) {
    return payload;
  }

  if (Array.isArray(payload)) {
    return Uint8Array.from(payload);
  }

  throw new Error("Invalid binary payload from Tauri channel.");
};

const toArrayBuffer = (bytes: Uint8Array): ArrayBuffer => {
  if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
    return bytes.buffer as ArrayBuffer;
  }
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
};

export const processTauriScannerPostProcessSourceFile = async (
  sourceFile: Blob,
  options: {
    documentPoints: Point[] | null;
    outputRotation: OrthogonalRotation;
    imageEnhancement: boolean;
    colorMode?: "auto" | "color" | "grayscale" | "binary";
    postprocessBackend?: ScannerPostProcessBackend;
    spineFlattening?: boolean;
    perspectiveTransform?: boolean;
    gridPostprocess?: "none" | "x-stretch-equalize";
    pipelineDebug?: boolean;
  },
): Promise<TauriScannerPostProcessResult> => {
  if (!isTauri()) {
    throw new Error("Native scanner post-process is only available in Tauri desktop builds.");
  }

  const sourceBytes = new Uint8Array(await sourceFile.arrayBuffer());
  const {invoke, Channel} = await import("@tauri-apps/api/core");

  return await new Promise<TauriScannerPostProcessResult>((resolve, reject) => {
    let settled = false;
    let overallTimeoutId: ReturnType<typeof setTimeout> | null = null;
    let partialTimeoutId: ReturnType<typeof setTimeout> | null = null;
    let response:
      | Omit<TauriScannerPostProcessResult, "encodedBytes">
      | null = null;
    let encodedBytes: ArrayBuffer | null = null;

    const clearTimers = (): void => {
      if (overallTimeoutId !== null) {
        clearTimeout(overallTimeoutId);
        overallTimeoutId = null;
      }

      if (partialTimeoutId !== null) {
        clearTimeout(partialTimeoutId);
        partialTimeoutId = null;
      }
    };

    const settleReject = (error: unknown): void => {
      if (settled) return;
      settled = true;
      clearTimers();
      reject(error instanceof Error ? error : new Error(String(error)));
    };

    const armPartialTimeout = (message: string): void => {
      if (settled || partialTimeoutId !== null || (response && encodedBytes !== null)) {
        return;
      }

      partialTimeoutId = setTimeout(() => {
        settleReject(new Error(message));
      }, NATIVE_POST_PROCESS_PARTIAL_TIMEOUT_MS);
    };

    const maybeResolve = (): void => {
      if (settled || !response || encodedBytes === null) {
        return;
      }

      settled = true;
      clearTimers();
      resolve({
        ...response,
        encodedBytes,
      });
    };

    overallTimeoutId = setTimeout(() => {
      settleReject(new Error("Native scanner post-process timed out."));
    }, NATIVE_POST_PROCESS_TIMEOUT_MS);

    const payloadChannel = new Channel<TauriRawChannelPayload>((message) => {
      try {
        encodedBytes = toArrayBuffer(normalizeTauriRawChannelPayload(message));
        if (!response) {
          armPartialTimeout("Native scanner post-process returned raw payload without metadata.");
        }
        maybeResolve();
      } catch (error) {
        settleReject(error);
      }
    });

    void invoke<Omit<TauriScannerPostProcessResult, "encodedBytes">>(
      "tauri_scanner_postprocess_image",
      {
        sourceBytes,
        request: {
          documentPoints: options.documentPoints,
          outputRotation: options.outputRotation,
          imageEnhancement: options.imageEnhancement,
          colorMode: options.colorMode ?? "auto",
          postprocessBackend: options.postprocessBackend ?? "heuristic",
          spineFlattening: options.spineFlattening ?? true,
          perspectiveTransform: options.perspectiveTransform ?? true,
          gridPostprocess: options.gridPostprocess ?? "none",
          pipelineDebug: options.pipelineDebug ?? false,
        },
        payloadChannel,
      },
    )
      .then((result) => {
        response = result;
        if (encodedBytes === null) {
          armPartialTimeout("Native scanner post-process returned metadata without raw payload.");
        }
        maybeResolve();
      })
      .catch((error) => {
        settleReject(error);
      });
  });
};

export const refineDocumentCorners = async (
  sourceFile: Blob,
  documentPoints: Point[],
): Promise<Point[]> => {
  if (!isTauri()) {
    throw new Error("Native corner refinement is only available in Tauri desktop builds.");
  }
  const sourceBytes = new Uint8Array(await sourceFile.arrayBuffer());
  const {invoke} = await import("@tauri-apps/api/core");
  return invoke<Point[]>("tauri_scanner_refine_document_corners", {
    sourceBytes,
    request: {
      documentPoints,
    },
  });
};

export async function fetchScannerAssetsStatus(): Promise<ScannerAssetsStatus> {
  return invokeTauriCommand<ScannerAssetsStatus>("scanner_assets_status");
}

export async function fetchScannerOrtProbe(): Promise<ScannerOrtProbeStatus> {
  return invokeTauriCommand<ScannerOrtProbeStatus>("scanner_probe_ort");
}

export async function checkScannerAssetsUpdate(): Promise<ScannerAssetsUpdateCheck> {
  return invokeTauriCommand<ScannerAssetsUpdateCheck>("scanner_assets_check_update");
}

export async function startScannerAssetsDownload(
  operationId: string,
  onProgress: (progress: ScannerAssetsProgress) => void,
): Promise<ScannerAssetsInstallResult> {
  if (!isTauri()) {
    throw new Error("Scanner commands are only available in Tauri desktop builds.");
  }
  const {invoke, Channel} = await import("@tauri-apps/api/core");
  const channel = new Channel<ScannerAssetsProgress>();
  channel.onmessage = onProgress;
  return await invoke<ScannerAssetsInstallResult>("scanner_assets_download", {
    request: {operationId},
    progressChannel: channel,
  });
}

export async function startScannerAssetsUpdateDownload(
  operationId: string,
  onProgress: (progress: ScannerAssetsProgress) => void,
): Promise<ScannerAssetsInstallResult> {
  if (!isTauri()) {
    throw new Error("Scanner commands are only available in Tauri desktop builds.");
  }
  const {invoke, Channel} = await import("@tauri-apps/api/core");
  const channel = new Channel<ScannerAssetsProgress>();
  channel.onmessage = onProgress;
  return await invoke<ScannerAssetsInstallResult>("scanner_assets_download_update", {
    request: {operationId},
    progressChannel: channel,
  });
}

export async function cancelScannerAssetsOperation(operationId: string): Promise<void> {
  return await invokeTauriCommand<void>("scanner_assets_cancel", {
    request: {operationId},
  });
}

export async function clearScannerAssets(): Promise<void> {
  return await invokeTauriCommand<void>("scanner_assets_clear");
}

export async function startScannerAssetsImport(
  archivePath: string,
  onProgress: (progress: ScannerAssetsProgress) => void,
): Promise<ScannerAssetsInstallResult> {
  if (!isTauri()) {
    throw new Error("Scanner commands are only available in Tauri desktop builds.");
  }
  const {invoke, Channel} = await import("@tauri-apps/api/core");
  const channel = new Channel<ScannerAssetsProgress>();
  channel.onmessage = onProgress;
  return await invoke<ScannerAssetsInstallResult>("scanner_assets_import", {
    request: {archivePath},
    progressChannel: channel,
  });
}

export function scannerErrorI18nKey(code: string): `error.codes.${string}` {
  return `error.codes.${code}`;
}
