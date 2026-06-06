import {create} from "zustand";
import type {
  ScannerAssetsError,
  ScannerAssetsProgress,
  ScannerAssetsStatus,
  ScannerOrtProbeStatus,
} from "../lib/tauri/scanner";
import {
  cancelScannerAssetsOperation,
  checkScannerAssetsUpdate,
  clearScannerAssets,
  fetchScannerAssetsStatus,
  fetchScannerOrtProbe,
  startScannerAssetsDownload,
  startScannerAssetsImport,
  startScannerAssetsUpdateDownload,
} from "../lib/tauri/scanner";

const CANCELLED_ERROR_CODE = "assets.operation.cancelled";

type ScannerOperationContext =
  | {kind: "download"; operationId: string; source: "default" | "update"}
  | {kind: "import"; archivePath: string}
  | {kind: "clear"};

export type ScannerUpdateCheckResult =
  | {kind: "up-to-date"; version: string}
  | {kind: "update-available"; version: string}
  | {kind: "install-available"; version: string};

export interface ScannerStore {
  assetsStatus: ScannerAssetsStatus | null;
  probeStatus: ScannerOrtProbeStatus | null;
  progress: ScannerAssetsProgress | null;
  isOperating: boolean;
  activeOperation: ScannerOperationContext | null;
  operationError: ScannerAssetsError | null;
  retryOperation: ScannerOperationContext | null;
  canRetryLastOperation: boolean;
  canCancelCurrentOperation: boolean;
  updateCheckResult: ScannerUpdateCheckResult | null;

  fetchStatus: () => Promise<void>;
  fetchProbe: () => Promise<void>;
  startDownload: () => Promise<void>;
  startUpdateDownload: () => Promise<void>;
  startImport: (archivePath: string) => Promise<void>;
  checkForAssetUpdate: () => Promise<void>;
  clearInstalledAssets: () => Promise<void>;
  cancelCurrentOperation: (options?: {silent?: boolean}) => Promise<void>;
  retryLastOperation: () => Promise<void>;
  clearError: () => void;
  clearUpdateCheckResult: () => void;
}

export const useScannerStore = create<ScannerStore>()((set, get) => ({
  assetsStatus: null,
  probeStatus: null,
  progress: null,
  isOperating: false,
  activeOperation: null,
  operationError: null,
  retryOperation: null,
  canRetryLastOperation: false,
  canCancelCurrentOperation: false,
  updateCheckResult: null,

  fetchStatus: async () => {
    try {
      const status = await fetchScannerAssetsStatus();
      set({assetsStatus: status});
    } catch (error) {
      const scannerError = asScannerError(error);
      set(displayOnlyErrorState(scannerError));
    }
  },

  fetchProbe: async () => {
    try {
      const probe = await fetchScannerOrtProbe();
      set({probeStatus: probe});
    } catch (error) {
      const scannerError = asScannerError(error);
      set(displayOnlyErrorState(scannerError));
    }
  },

  startDownload: async () => {
    if (get().isOperating) return;
    const operation = {
      kind: "download",
      operationId: createOperationId(),
      source: "default",
    } as const;
    set({
      isOperating: true,
      activeOperation: operation,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
      canCancelCurrentOperation: true,
      updateCheckResult: null,
    });
    try {
      await startScannerAssetsDownload(operation.operationId, (progress) => {
        if (!operationMatches(get().activeOperation, operation)) return;
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          if (isCancellationError(progress.error)) {
            set(operationCancelledState());
            return;
          }
          set(operationFailureState(progress.error, operation));
        }
      });
      if (!operationMatches(get().activeOperation, operation)) return;
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().activeOperation, operation)) return;
      const scannerError = asScannerError(error);
      if (isCancellationError(scannerError)) {
        set(operationCancelledState());
        return;
      }
      set(operationFailureState(scannerError, operation));
    }
  },

  startUpdateDownload: async () => {
    if (get().isOperating) return;
    const operation = {
      kind: "download",
      operationId: createOperationId(),
      source: "update",
    } as const;
    set({
      isOperating: true,
      activeOperation: operation,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
      canCancelCurrentOperation: true,
      updateCheckResult: null,
    });
    try {
      await startScannerAssetsUpdateDownload(operation.operationId, (progress) => {
        if (!operationMatches(get().activeOperation, operation)) return;
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          if (isCancellationError(progress.error)) {
            set(operationCancelledState());
            return;
          }
          set(operationFailureState(progress.error, operation));
        }
      });
      if (!operationMatches(get().activeOperation, operation)) return;
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().activeOperation, operation)) return;
      const scannerError = asScannerError(error);
      if (isCancellationError(scannerError)) {
        set(operationCancelledState());
        return;
      }
      set(operationFailureState(scannerError, operation));
    }
  },

  startImport: async (archivePath: string) => {
    if (get().isOperating) return;
    const operation = {kind: "import", archivePath} as const;
    set({
      isOperating: true,
      activeOperation: operation,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
      canCancelCurrentOperation: false,
      updateCheckResult: null,
    });
    try {
      await startScannerAssetsImport(archivePath, (progress) => {
        if (!operationMatches(get().activeOperation, operation)) return;
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          set(operationFailureState(progress.error, operation));
        }
      });
      if (!operationMatches(get().activeOperation, operation)) return;
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().activeOperation, operation)) return;
      const scannerError = asScannerError(error);
      set(operationFailureState(scannerError, operation));
    }
  },

  checkForAssetUpdate: async () => {
    if (get().isOperating) return;
    try {
      const status = await fetchScannerAssetsStatus();
      const update = await checkScannerAssetsUpdate();
      set({
        assetsStatus: status,
        operationError: null,
        progress: null,
        retryOperation: null,
        canRetryLastOperation: false,
        canCancelCurrentOperation: false,
      });

      if (
        !update.updateAvailable
      ) {
        set({updateCheckResult: {kind: "up-to-date", version: update.targetAssetTag}});
        return;
      }

      if (status.state === "ready" && status.manifest) {
        set({
          updateCheckResult: {
            kind: "update-available",
            version: update.targetAssetTag,
          },
        });
        return;
      }

      set({
        updateCheckResult: {
          kind: "install-available",
          version: update.targetAssetTag,
        },
      });
    } catch (error) {
      const scannerError = asScannerError(error);
      set({...displayOnlyErrorState(scannerError), updateCheckResult: null});
    }
  },

  clearInstalledAssets: async () => {
    if (get().isOperating) return;
    const operation = {kind: "clear"} as const;
    set({
      isOperating: true,
      activeOperation: operation,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
      canCancelCurrentOperation: false,
      updateCheckResult: null,
    });
    try {
      await clearScannerAssets();
      if (!operationMatches(get().activeOperation, operation)) return;
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      if (!operationMatches(get().activeOperation, operation)) return;
      const scannerError = asScannerError(error);
      set(operationFailureState(scannerError, operation));
    }
  },

  cancelCurrentOperation: async () => {
    const operation = get().activeOperation;
    if (!operation || operation.kind !== "download" || !get().canCancelCurrentOperation) return;

    set({
      operationError: null,
      retryOperation: null,
      canRetryLastOperation: false,
      canCancelCurrentOperation: false,
      updateCheckResult: null,
    });
    try {
      await cancelScannerAssetsOperation(operation.operationId);
    } catch {
      // Cancellation is best-effort and idempotent; the active operation will settle normally.
    }
  },

  retryLastOperation: async () => {
    const operation = get().retryOperation;
    if (!operation || get().isOperating) return;

    if (operation.kind === "download") {
      if (operation.source === "update") {
        await get().startUpdateDownload();
        return;
      }
      await get().startDownload();
      return;
    }

    if (operation.kind === "import") {
      await get().startImport(operation.archivePath);
      return;
    }

    await get().clearInstalledAssets();
  },

  clearError: () =>
    set({
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
      updateCheckResult: null,
    }),

  clearUpdateCheckResult: () => set({updateCheckResult: null}),
}));

function operationSuccessState() {
  return {
    isOperating: false,
    activeOperation: null,
    progress: null,
    operationError: null,
    retryOperation: null,
    canRetryLastOperation: false,
    canCancelCurrentOperation: false,
    updateCheckResult: null,
  };
}

function displayOnlyErrorState(error: ScannerAssetsError) {
  return {
    isOperating: false,
    activeOperation: null,
    progress: null,
    operationError: error,
    retryOperation: null,
    canRetryLastOperation: false,
    canCancelCurrentOperation: false,
  };
}

function operationFailureState(
  error: ScannerAssetsError,
  operation: ScannerOperationContext,
) {
  const retryOperation = error.retryable && !isCancellationError(error) ? operation : null;
  return {
    isOperating: false,
    activeOperation: null,
    operationError: error,
    retryOperation,
    canRetryLastOperation: retryOperation != null,
    canCancelCurrentOperation: false,
    updateCheckResult: null,
  };
}

function operationCancelledState() {
  return {
    isOperating: false,
    activeOperation: null,
    progress: null,
    operationError: null,
    retryOperation: null,
    canRetryLastOperation: false,
    canCancelCurrentOperation: false,
    updateCheckResult: null,
  };
}

function operationMatches(
  activeOperation: ScannerOperationContext | null,
  operation: ScannerOperationContext,
) {
  if (!activeOperation || activeOperation.kind !== operation.kind) return false;
  if (operation.kind === "download") {
    return (
      activeOperation.kind === "download" &&
      activeOperation.operationId === operation.operationId &&
      activeOperation.source === operation.source
    );
  }
  if (operation.kind === "import") {
    return activeOperation.kind === "import" && activeOperation.archivePath === operation.archivePath;
  }
  return activeOperation.kind === "clear";
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
