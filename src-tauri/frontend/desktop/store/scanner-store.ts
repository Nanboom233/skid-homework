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

export interface ScannerStore {
  assetsStatus: ScannerAssetsStatus | null;
  probeStatus: ScannerOrtProbeStatus | null;
  progress: ScannerAssetsProgress | null;
  isOperating: boolean;
  operationError: ScannerAssetsError | null;

  fetchStatus: () => Promise<void>;
  fetchProbe: () => Promise<void>;
  startDownload: () => Promise<void>;
  startImport: (archivePath: string) => Promise<void>;
  clearError: () => void;
}

export const useScannerStore = create<ScannerStore>()((set, get) => ({
  assetsStatus: null,
  probeStatus: null,
  progress: null,
  isOperating: false,
  operationError: null,

  fetchStatus: async () => {
    try {
      const status = await fetchScannerAssetsStatus();
      set({assetsStatus: status});
    } catch (error) {
      const scannerError = asScannerError(error);
      set({operationError: scannerError});
    }
  },

  fetchProbe: async () => {
    try {
      const probe = await fetchScannerOrtProbe();
      set({probeStatus: probe});
    } catch (error) {
      const scannerError = asScannerError(error);
      set({operationError: scannerError});
    }
  },

  startDownload: async () => {
    if (get().isOperating) return;
    set({isOperating: true, operationError: null, progress: null});
    try {
      await startScannerAssetsDownload((progress) => {
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          set({operationError: progress.error, isOperating: false});
        }
      });
      set({isOperating: false, progress: null});
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      const scannerError = asScannerError(error);
      set({isOperating: false, operationError: scannerError});
    }
  },

  startImport: async (archivePath: string) => {
    if (get().isOperating) return;
    set({isOperating: true, operationError: null, progress: null});
    try {
      await startScannerAssetsImport(archivePath, (progress) => {
        set({progress});
        if (progress.phase === "failed" && progress.error) {
          set({operationError: progress.error, isOperating: false});
        }
      });
      set({isOperating: false, progress: null});
      await get().fetchStatus();
      await get().fetchProbe();
    } catch (error) {
      const scannerError = asScannerError(error);
      set({isOperating: false, operationError: scannerError});
    }
  },

  clearError: () => set({operationError: null}),
}));

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
