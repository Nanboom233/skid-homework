import {isTauri} from "./platform";

export type AssetTarget = "onnxruntime" | "camera-server";

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

export interface ScannerCameraAssetSummary {
  assetVersion: string;
  path: string;
}

export interface OrtAssetStatus {
  state: string;
  platformTarget: string;
  currentDir?: string;
  manifest?: ScannerAssetsManifestSummary;
  lastError?: ScannerAssetsError;
}

export interface CameraAssetStatus {
  state: string;
  currentDir?: string;
  artifact?: ScannerCameraAssetSummary;
  lastError?: ScannerAssetsError;
}

export interface ScannerAssetsStatusResponse {
  onnxruntime: OrtAssetStatus;
  "camera-server": CameraAssetStatus;
}

export interface OrtUpdateCheck {
  platformTarget: string;
  currentAssetVersion?: string;
  targetAssetTag: string;
  updateAvailable: boolean;
}

export interface CameraUpdateCheck {
  currentAssetVersion?: string;
  targetAssetTag: string;
  updateAvailable: boolean;
}

export interface ScannerAssetsUpdateCheckResponse {
  onnxruntime: OrtUpdateCheck;
  "camera-server": CameraUpdateCheck;
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
  platformTarget?: string;
  currentDir: string;
  path?: string;
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

export async function fetchScannerAssetsStatus(): Promise<ScannerAssetsStatusResponse> {
  return invokeTauriCommand<ScannerAssetsStatusResponse>("scanner_assets_status");
}

export async function fetchScannerOrtProbe(): Promise<ScannerOrtProbeStatus> {
  return invokeTauriCommand<ScannerOrtProbeStatus>("scanner_probe_ort");
}

export async function checkScannerAssetsUpdate(): Promise<ScannerAssetsUpdateCheckResponse> {
  return invokeTauriCommand<ScannerAssetsUpdateCheckResponse>("scanner_assets_check_update");
}

export async function startScannerAssetDownload(
  target: AssetTarget,
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
    request: {target, operationId},
    progressChannel: channel,
  });
}

export async function startScannerAssetImport(
  target: AssetTarget,
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
    request: {target, archivePath},
    progressChannel: channel,
  });
}

export async function cancelScannerAssetOperation(operationId: string): Promise<void> {
  return await invokeTauriCommand<void>("scanner_assets_cancel", {
    request: {operationId},
  });
}

export async function clearScannerAssets(target: AssetTarget): Promise<void> {
  return await invokeTauriCommand<void>("scanner_assets_clear", {
    request: {target},
  });
}

export function scannerErrorI18nKey(code: string): `error.codes.${string}` {
  return `error.codes.${code}`;
}
