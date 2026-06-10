import {create} from "zustand";
import type {
  AssetTarget,
  ScannerAssetsError,
  ScannerAssetsProgress,
  ScannerAssetsStatusResponse,
  ScannerAssetsUpdateCheckResponse,
  ScannerOrtProbeStatus,
} from "../lib/tauri/scanner";
import {
  cancelScannerAssetOperation,
  checkScannerAssetsUpdate,
  clearScannerAssets,
  fetchScannerAssetsStatus,
  fetchScannerOrtProbe,
  startScannerAssetDownload,
  startScannerAssetImport,
} from "../lib/tauri/scanner";

const CANCELLED_ERROR_CODE = "assets.operation.cancelled";

export type ScannerOperationContext =
  | {kind: "download"; target: AssetTarget; operationId: string}
  | {kind: "import"; target: AssetTarget; archivePath: string}
  | {kind: "clear"; target: AssetTarget};

export interface ScannerOperationState {
  progress: ScannerAssetsProgress | null;
  isOperating: boolean;
  active: ScannerOperationContext | null;
  error: ScannerAssetsError | null;
  retry: ScannerOperationContext | null;
  canRetry: boolean;
  canCancel: boolean;
}

export interface ScannerStore {
  assetsStatus: ScannerAssetsStatusResponse | null;
  probeStatus: ScannerOrtProbeStatus | null;
  operation: ScannerOperationState;
  updateCheckResult: ScannerAssetsUpdateCheckResponse | null;

  fetchStatus: () => Promise<void>;
  fetchProbe: () => Promise<void>;
  checkForUpdate: () => Promise<void>;
  startDownload: (target: AssetTarget) => Promise<void>;
  startImport: (target: AssetTarget, archivePath: string) => Promise<void>;
  startUpdateAll: () => Promise<void>;
  clearAssets: (target: AssetTarget) => Promise<void>;
  cancelCurrentOperation: () => Promise<void>;
  retryLastOperation: () => Promise<void>;
  clearError: () => void;
}

const initialOperationState: ScannerOperationState = {
  progress: null,
  isOperating: false,
  active: null,
  error: null,
  retry: null,
  canRetry: false,
  canCancel: false,
};

export const useScannerStore = create<ScannerStore>()((set, get) => ({
  assetsStatus: null,
  probeStatus: null,
  operation: {...initialOperationState},
  updateCheckResult: null,

  fetchStatus: async () => {
    try {
      const status = await fetchScannerAssetsStatus();
      set({assetsStatus: status});
    } catch (error) {
      const scannerError = asScannerError(error);
      set({operation: displayOnlyErrorState(scannerError)});
    }
  },

  fetchProbe: async () => {
    try {
      const probe = await fetchScannerOrtProbe();
      set({probeStatus: probe});
    } catch (error) {
      const scannerError = asScannerError(error);
      set({operation: displayOnlyErrorState(scannerError)});
    }
  },

  checkForUpdate: async () => {
    if (get().operation.isOperating) return;
    try {
      const status = await fetchScannerAssetsStatus();
      const update = await checkScannerAssetsUpdate();
      set({
        assetsStatus: status,
        updateCheckResult: update,
        operation: {...initialOperationState},
      });
    } catch (error) {
      const scannerError = asScannerError(error);
      set({
        operation: displayOnlyErrorState(scannerError),
      });
    }
  },

  startDownload: async (target: AssetTarget) => {
    if (get().operation.isOperating) return;
    const operation: ScannerOperationContext = {
      kind: "download",
      target,
      operationId: createOperationId(),
    };
    set({
      operation: {
        ...initialOperationState,
        isOperating: true,
        active: operation,
        canCancel: true,
      },
    });
    try {
      const result = await startScannerAssetDownload(target, operation.operationId, (progress) => {
        if (!operationMatches(get().operation.active, operation)) return;
        set({operation: {...get().operation, progress}});
        if (progress.phase === "failed" && progress.error) {
          if (isCancellationError(progress.error)) {
            set({operation: operationCancelledState()});
            return;
          }
          set({operation: operationFailureState(progress.error, operation)});
        }
      });
      if (!operationMatches(get().operation.active, operation)) return;
      set((state) => ({
        assetsStatus: mergeInstalledAssetStatus(state.assetsStatus, target, result),
        updateCheckResult: markTargetCurrent(state.updateCheckResult, target, result.assetVersion),
        operation: operationSuccessState(),
      }));
      await get().checkForUpdate();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().operation.active, operation)) return;
      const scannerError = asScannerError(error);
      if (isCancellationError(scannerError)) {
        set({operation: operationCancelledState()});
        return;
      }
      set({operation: operationFailureState(scannerError, operation)});
    }
  },

  startImport: async (target: AssetTarget, archivePath: string) => {
    if (get().operation.isOperating) return;
    const operation: ScannerOperationContext = {kind: "import", target, archivePath};
    set({
      operation: {
        ...initialOperationState,
        isOperating: true,
        active: operation,
      },
    });
    try {
      const result = await startScannerAssetImport(target, archivePath, (progress) => {
        if (!operationMatches(get().operation.active, operation)) return;
        set({operation: {...get().operation, progress}});
        if (progress.phase === "failed" && progress.error) {
          set({operation: operationFailureState(progress.error, operation)});
        }
      });
      if (!operationMatches(get().operation.active, operation)) return;
      set((state) => ({
        assetsStatus: mergeInstalledAssetStatus(state.assetsStatus, target, result),
        operation: operationSuccessState(),
      }));
      await get().checkForUpdate();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().operation.active, operation)) return;
      const scannerError = asScannerError(error);
      set({operation: operationFailureState(scannerError, operation)});
    }
  },

  startUpdateAll: async () => {
    if (get().operation.isOperating) return;
    const check = get().updateCheckResult;
    if (!check) return;

    const queue: AssetTarget[] = [];
    if (check["camera-server"].updateAvailable) queue.push("camera-server");
    if (check.onnxruntime.updateAvailable) queue.push("onnxruntime");
    if (queue.length === 0) return;

    for (const target of queue) {
      if (get().operation.error) break;
      const operation: ScannerOperationContext = {
        kind: "download",
        target,
        operationId: createOperationId(),
      };
      set({
        operation: {
          ...initialOperationState,
          isOperating: true,
          active: operation,
          canCancel: true,
        },
      });
      try {
        const result = await startScannerAssetDownload(target, operation.operationId, (progress) => {
          if (!operationMatches(get().operation.active, operation)) return;
          set({operation: {...get().operation, progress}});
          if (progress.phase === "failed" && progress.error) {
            if (isCancellationError(progress.error)) {
              set({operation: operationCancelledState()});
              return;
            }
            set({operation: operationFailureState(progress.error, operation)});
          }
        });
        if (!operationMatches(get().operation.active, operation)) break;
        set((state) => ({
          assetsStatus: mergeInstalledAssetStatus(state.assetsStatus, target, result),
          updateCheckResult: markTargetCurrent(state.updateCheckResult, target, result.assetVersion),
          operation: operationSuccessState(),
        }));
      } catch (error) {
        if (!operationMatches(get().operation.active, operation)) break;
        const scannerError = asScannerError(error);
        if (isCancellationError(scannerError)) {
          set({operation: operationCancelledState()});
          break;
        }
        set({operation: operationFailureState(scannerError, operation)});
        break;
      }
    }
    if (get().operation.error) {
      await get().fetchStatus();
    } else {
      await get().checkForUpdate();
    }
    await get().fetchProbe();
  },

  clearAssets: async (target: AssetTarget) => {
    if (get().operation.isOperating) return;
    const operation: ScannerOperationContext = {kind: "clear", target};
    set({
      operation: {
        ...initialOperationState,
        isOperating: true,
        active: operation,
      },
    });
    try {
      await clearScannerAssets(target);
      if (!operationMatches(get().operation.active, operation)) return;
      set((state) => ({
        assetsStatus: markTargetMissing(state.assetsStatus, target),
        updateCheckResult: markTargetMissingForUpdate(state.updateCheckResult, target),
        operation: operationSuccessState(),
      }));
      await get().checkForUpdate();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().operation.active, operation)) return;
      const scannerError = asScannerError(error);
      set({operation: operationFailureState(scannerError, operation)});
    }
  },

  cancelCurrentOperation: async () => {
    const active = get().operation.active;
    if (!active || !get().operation.canCancel) return;
    if (active.kind !== "download") return;

    set({
      operation: {
        ...get().operation,
        canCancel: false,
      },
    });
    try {
      await cancelScannerAssetOperation(active.operationId);
    } catch {
      // Best-effort
    }
  },

  retryLastOperation: async () => {
    const retry = get().operation.retry;
    if (!retry || get().operation.isOperating) return;

    if (retry.kind === "download") {
      await get().startDownload(retry.target);
      return;
    }
    if (retry.kind === "import") {
      await get().startImport(retry.target, retry.archivePath);
      return;
    }
    if (retry.kind === "clear") {
      await get().clearAssets(retry.target);
    }
  },

  clearError: () =>
    set({
      operation: {...initialOperationState},
    }),
}));

function operationSuccessState(): ScannerOperationState {
  return {...initialOperationState};
}

function displayOnlyErrorState(error: ScannerAssetsError): ScannerOperationState {
  return {
    ...initialOperationState,
    error,
  };
}

function operationFailureState(
  error: ScannerAssetsError,
  operation: ScannerOperationContext,
): ScannerOperationState {
  const retry = error.retryable && !isCancellationError(error) ? operation : null;
  return {
    ...initialOperationState,
    error,
    retry,
    canRetry: retry != null,
  };
}

function operationCancelledState(): ScannerOperationState {
  return {...initialOperationState};
}

function mergeInstalledAssetStatus(
  status: ScannerAssetsStatusResponse | null,
  target: AssetTarget,
  result: {assetVersion: string; platformTarget?: string; currentDir: string; path?: string},
): ScannerAssetsStatusResponse | null {
  if (!status) return status;

  if (target === "camera-server") {
    return {
      ...status,
      "camera-server": {
        state: "ready",
        currentDir: result.currentDir,
        artifact: {
          assetVersion: result.assetVersion,
          path: result.path ?? status["camera-server"].artifact?.path ?? result.currentDir,
        },
      },
    };
  }

  const platformTarget = result.platformTarget ?? status.onnxruntime.platformTarget;
  return {
    ...status,
    onnxruntime: {
      state: "ready",
      platformTarget,
      currentDir: result.currentDir,
      manifest: {
        schemaVersion: status.onnxruntime.manifest?.schemaVersion ?? 1,
        assetVersion: result.assetVersion,
        platformTarget,
      },
    },
  };
}

function markTargetMissing(
  status: ScannerAssetsStatusResponse | null,
  target: AssetTarget,
): ScannerAssetsStatusResponse | null {
  if (!status) return status;

  if (target === "camera-server") {
    return {
      ...status,
      "camera-server": {
        state: "missing",
        currentDir: status["camera-server"].currentDir,
      },
    };
  }

  return {
    ...status,
    onnxruntime: {
      state: "missing",
      platformTarget: status.onnxruntime.platformTarget,
      currentDir: status.onnxruntime.currentDir,
    },
  };
}

function markTargetCurrent(
  update: ScannerAssetsUpdateCheckResponse | null,
  target: AssetTarget,
  assetVersion: string,
): ScannerAssetsUpdateCheckResponse | null {
  if (!update) return update;

  if (target === "camera-server") {
    return {
      ...update,
      "camera-server": {
        ...update["camera-server"],
        currentAssetVersion: assetVersion,
        updateAvailable: false,
      },
    };
  }

  return {
    ...update,
    onnxruntime: {
      ...update.onnxruntime,
      currentAssetVersion: assetVersion,
      updateAvailable: false,
    },
  };
}

function markTargetMissingForUpdate(
  update: ScannerAssetsUpdateCheckResponse | null,
  target: AssetTarget,
): ScannerAssetsUpdateCheckResponse | null {
  if (!update) return update;

  if (target === "camera-server") {
    const {currentAssetVersion: _currentAssetVersion, ...cameraUpdate} = update["camera-server"];
    void _currentAssetVersion;
    return {
      ...update,
      "camera-server": {
        ...cameraUpdate,
        updateAvailable: true,
      },
    };
  }

  const {currentAssetVersion: _currentAssetVersion, ...ortUpdate} = update.onnxruntime;
  void _currentAssetVersion;
  return {
    ...update,
    onnxruntime: {
      ...ortUpdate,
      updateAvailable: true,
    },
  };
}

function operationMatches(
  active: ScannerOperationContext | null,
  operation: ScannerOperationContext,
) {
  if (!active || active.kind !== operation.kind) return false;
  if (operation.kind === "download" && active.kind === "download") {
    return active.operationId === operation.operationId && active.target === operation.target;
  }
  if (operation.kind === "import" && active.kind === "import") {
    return active.archivePath === operation.archivePath && active.target === operation.target;
  }
  if (operation.kind === "clear" && active.kind === "clear") {
    return active.target === operation.target;
  }
  return false;
}

function isCancellationError(error: ScannerAssetsError) {
  return error.code === CANCELLED_ERROR_CODE;
}

function createOperationId() {
  if (globalThis.crypto?.randomUUID) {
    return globalThis.crypto.randomUUID();
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function asScannerError(error: unknown): ScannerAssetsError {
  if (
    error &&
    typeof error === "object" &&
    "code" in error &&
    typeof (error as Record<string, unknown>).code === "string"
  ) {
    return error as ScannerAssetsError;
  }
  return {
    code: "unknown",
    retryable: true,
    details: error instanceof Error ? error.message : String(error),
  };
}
