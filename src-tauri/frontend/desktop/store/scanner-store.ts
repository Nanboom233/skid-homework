import {create} from "zustand";
import type {
  ScannerAssetsError,
  ScannerAssetsProgress,
  ScannerAssetsStatus,
  ScannerOrtProbeStatus,
} from "../lib/tauri/scanner";
import {
  fetchScannerAssetsStatus,
  fetchScannerOrtProbe,
  startScannerAssetsDownload,
  startScannerAssetsImport,
} from "../lib/tauri/scanner";

type ScannerOperationContext =
  | {kind: "download"}
  | {kind: "import"; archivePath: string};

export interface ScannerStore {
  assetsStatus: ScannerAssetsStatus | null;
  probeStatus: ScannerOrtProbeStatus | null;
  progress: ScannerAssetsProgress | null;
  isOperating: boolean;
  operationError: ScannerAssetsError | null;
  retryOperation: ScannerOperationContext | null;
  canRetryLastOperation: boolean;

  fetchStatus: () => Promise<void>;
  fetchProbe: () => Promise<void>;
  startDownload: () => Promise<void>;
  startImport: (archivePath: string) => Promise<void>;
  retryLastOperation: () => Promise<void>;
  clearError: () => void;
}

export const useScannerStore = create<ScannerStore>()((set, get) => ({
  assetsStatus: null,
  probeStatus: null,
  progress: null,
  isOperating: false,
  operationError: null,
  retryOperation: null,
  canRetryLastOperation: false,

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
    const operation = {kind: "download"} as const;
    set({
      isOperating: true,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
    });
    try {
      await startScannerAssetsDownload((progress) => {
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          set(operationFailureState(progress.error, operation));
        }
      });
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      const scannerError = asScannerError(error);
      set(operationFailureState(scannerError, operation));
    }
  },

  startImport: async (archivePath: string) => {
    if (get().isOperating) return;
    const operation = {kind: "import", archivePath} as const;
    set({
      isOperating: true,
      operationError: null,
      progress: null,
      retryOperation: null,
      canRetryLastOperation: false,
    });
    try {
      await startScannerAssetsImport(archivePath, (progress) => {
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          set(operationFailureState(progress.error, operation));
        }
      });
      set(operationSuccessState());
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      const scannerError = asScannerError(error);
      set(operationFailureState(scannerError, operation));
    }
  },

  retryLastOperation: async () => {
    const operation = get().retryOperation;
    if (!operation || get().isOperating) return;

    if (operation.kind === "download") {
      await get().startDownload();
      return;
    }

    await get().startImport(operation.archivePath);
  },

  clearError: () =>
    set({
      operationError: null,
      retryOperation: null,
      canRetryLastOperation: false,
    }),
}));

function operationSuccessState() {
  return {
    isOperating: false,
    progress: null,
    operationError: null,
    retryOperation: null,
    canRetryLastOperation: false,
  };
}

function displayOnlyErrorState(error: ScannerAssetsError) {
  return {
    operationError: error,
    retryOperation: null,
    canRetryLastOperation: false,
  };
}

function operationFailureState(
  error: ScannerAssetsError,
  operation: ScannerOperationContext,
) {
  const retryOperation = error.retryable ? operation : null;
  return {
    isOperating: false,
    operationError: error,
    retryOperation,
    canRetryLastOperation: retryOperation != null,
  };
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
