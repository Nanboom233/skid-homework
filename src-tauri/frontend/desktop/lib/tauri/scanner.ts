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

export type ScannerErrorI18nKey =
  | "error.codes.assets.operation.cancelled"
  | "error.codes.assets.operation.inProgress"
  | "error.codes.assets.operation.taskFailed"
  | "error.codes.assets.clear.removeFailed"
  | "error.codes.assets.update.checkFailed"
  | "error.codes.assets.import.archive.openFailed"
  | "error.codes.assets.import.archive.formatMismatch"
  | "error.codes.assets.install.manifest.missing"
  | "error.codes.assets.install.manifest.invalid"
  | "error.codes.assets.install.manifest.versionMismatch"
  | "error.codes.assets.install.manifest.platformMismatch"
  | "error.codes.assets.install.verify.unsafePath"
  | "error.codes.assets.install.verify.fileMissing"
  | "error.codes.assets.install.verify.sizeMismatch"
  | "error.codes.assets.install.verify.checksumMismatch"
  | "error.codes.assets.install.activate.replaceFailed"
  | "error.codes.assets.download.fetch.networkFailed"
  | "error.codes.assets.download.fetch.httpStatus"
  | "error.codes.assets.download.fetch.writeFailed"
  | "error.codes.assets.status.resolveDataDirFailed"
  | "error.codes.runtime.probe.taskFailed";

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

export function scannerErrorI18nKey(code: string): ScannerErrorI18nKey | null {
  switch (code) {
    case "assets.operation.cancelled":
      return "error.codes.assets.operation.cancelled";
    case "assets.operation.inProgress":
      return "error.codes.assets.operation.inProgress";
    case "assets.operation.taskFailed":
      return "error.codes.assets.operation.taskFailed";
    case "assets.clear.removeFailed":
      return "error.codes.assets.clear.removeFailed";
    case "assets.update.checkFailed":
      return "error.codes.assets.update.checkFailed";
    case "assets.import.archive.openFailed":
      return "error.codes.assets.import.archive.openFailed";
    case "assets.import.archive.formatMismatch":
      return "error.codes.assets.import.archive.formatMismatch";
    case "assets.install.manifest.missing":
      return "error.codes.assets.install.manifest.missing";
    case "assets.install.manifest.invalid":
      return "error.codes.assets.install.manifest.invalid";
    case "assets.install.manifest.versionMismatch":
      return "error.codes.assets.install.manifest.versionMismatch";
    case "assets.install.manifest.platformMismatch":
      return "error.codes.assets.install.manifest.platformMismatch";
    case "assets.install.verify.unsafePath":
      return "error.codes.assets.install.verify.unsafePath";
    case "assets.install.verify.fileMissing":
      return "error.codes.assets.install.verify.fileMissing";
    case "assets.install.verify.sizeMismatch":
      return "error.codes.assets.install.verify.sizeMismatch";
    case "assets.install.verify.checksumMismatch":
      return "error.codes.assets.install.verify.checksumMismatch";
    case "assets.install.activate.replaceFailed":
      return "error.codes.assets.install.activate.replaceFailed";
    case "assets.download.fetch.networkFailed":
      return "error.codes.assets.download.fetch.networkFailed";
    case "assets.download.fetch.httpStatus":
      return "error.codes.assets.download.fetch.httpStatus";
    case "assets.download.fetch.writeFailed":
      return "error.codes.assets.download.fetch.writeFailed";
    case "assets.status.resolveDataDirFailed":
      return "error.codes.assets.status.resolveDataDirFailed";
    case "runtime.probe.taskFailed":
      return "error.codes.runtime.probe.taskFailed";
    default:
      return null;
  }
}
