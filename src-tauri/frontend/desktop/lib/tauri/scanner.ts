import {isTauri} from "./platform";

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
  sizeBytes?: number;
  required: boolean;
}

export type ScannerOrtResourceTreeEntry = {
  relativePath: string;
  name: string;
  depth: number;
  isDir: boolean;
  sizeBytes?: number;
};

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
  resourceTree: ScannerOrtResourceTreeEntry[];
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
