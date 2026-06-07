import {create} from "zustand";
import type {ScannerDetectionBackend} from "@/lib/scanner-types";
import type {FrameSource, Point, ScannerConfig} from "../lib/scanner";
import type {OrthogonalRotation} from "../lib/scanner/image-data";
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

export type ScannerStatus = "idle" | "connecting" | "streaming" | "error";
export type ScannerCapturedDocumentStatus = "processing" | "ready" | "failed";
export type ScannerReconnectState =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stopped"
  | "error";
export type ScannerHighQualityCaptureStatus =
  | "idle"
  | "capturing"
  | "processing"
  | "success"
  | "error";
export type ScannerCvPipeline = "idle" | "preview" | "single-hq";

export interface ScannerPreviewDebugState {
  frameIndex: number;
  previewFps: number | null;
  recentWindowFps: number | null;
  effectiveFps: number | null;
  payloadBytes: number | null;
  pollWaitMs: number | null;
  jsDecodeMs: number | null;
  canvasDrawMs: number | null;
  pollCount: number | null;
  previewWidth: number | null;
  previewHeight: number | null;
  transport: string | null;
  updatedAt: number | null;
}

export interface ScannerCvDebugState {
  pipeline: ScannerCvPipeline;
  cvReady: boolean;
  requestedBackend: ScannerDetectionBackend;
  activeBackend: ScannerDetectionBackend;
  strictMode: boolean;
  preferredProvider: string | null;
  preferredProviderReady: boolean;
  selectedModelId: string | null;
  selectedModelKind: string | null;
  selectedModelTask: string | null;
  backendMessage: string | null;
  documentDetected: boolean;
  cornerCount: number;
  cornerPoints: Point[];
  isStable: boolean;
  processingWidth: number | null;
  processingHeight: number | null;
  autoCaptureEnabled: boolean;
  isProcessing: boolean;
  updatedAt: number | null;
}

export interface ScannerConnectionDebugState {
  reconnectState: ScannerReconnectState;
  reconnectAttempt: number | null;
  reconnectMaxAttempts: number | null;
  reconnectDelayMs: number | null;
  reconnectMessage: string | null;
  lastErrorReason: string | null;
  lastDisconnectAt: number | null;
}

export interface ScannerCaptureDebugState {
  highQualityStatus: ScannerHighQualityCaptureStatus;
  highQualitySource: string | null;
  highQualityFallbackReason: string | null;
  lastCaptureSource: "preview-stream" | "single-hq" | null;
  lastCaptureWidth: number | null;
  lastCaptureHeight: number | null;
  lastCaptureAt: number | null;
  lastCaptureError: string | null;
  lastCaptureDocumentDetected: boolean;
}

export interface ScannerCapturedDocument {
  id: string;
  file: File;
  sourceFile: File;
  points: Point[] | null;
  status: ScannerCapturedDocumentStatus;
  error: string | null;
  documentDetected: boolean;
  captureSource: "preview-stream" | "single-hq";
  sourceWidth: number;
  sourceHeight: number;
  outputNameBase: string;
  outputRotation: OrthogonalRotation;
}

export type ScannerOperationContext =
  | {kind: "download"; operationId: string; source: "default" | "update"}
  | {kind: "import"; archivePath: string}
  | {kind: "clear"};

export type ScannerUpdateCheckResult =
  | {kind: "up-to-date"; version: string}
  | {kind: "update-available"; version: string}
  | {kind: "install-available"; version: string};

export interface ScannerStore {
  status: ScannerStatus;
  errorMessage: string | null;
  frameSource: FrameSource | null;
  config: ScannerConfig | null;
  frameCount: number;
  capturedDocuments: ScannerCapturedDocument[];
  previewDebug: ScannerPreviewDebugState;
  cvDebug: ScannerCvDebugState;
  connectionDebug: ScannerConnectionDebugState;
  captureDebug: ScannerCaptureDebugState;

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

  setStatus: (status: ScannerStatus, error?: string) => void;
  setFrameSource: (source: FrameSource | null) => void;
  setConfig: (config: ScannerConfig | null) => void;
  incrementFrameCount: () => void;
  resetFrameCount: () => void;
  addCapturedDocument: (doc: ScannerCapturedDocument) => void;
  updateCapturedDocument: (
    id: string,
    updates: Partial<ScannerCapturedDocument>,
  ) => void;
  removeCapturedDocument: (id: string) => void;
  clearCapturedDocuments: () => void;
  setPreviewDebug: (patch: Partial<ScannerPreviewDebugState>) => void;
  setCvDebug: (patch: Partial<ScannerCvDebugState>) => void;
  setConnectionDebug: (patch: Partial<ScannerConnectionDebugState>) => void;
  setCaptureDebug: (patch: Partial<ScannerCaptureDebugState>) => void;
  resetDebugState: () => void;
  reset: () => void;

  fetchStatus: () => Promise<void>;
  fetchProbe: () => Promise<void>;
  startDownload: () => Promise<void>;
  startUpdateDownload: () => Promise<void>;
  startImport: (archivePath: string) => Promise<void>;
  checkForAssetUpdate: () => Promise<void>;
  clearInstalledAssets: () => Promise<void>;
  cancelCurrentOperation: () => Promise<void>;
  retryLastOperation: () => Promise<void>;
  clearError: () => void;
  clearUpdateCheckResult: () => void;
}

export const useScannerStore = create<ScannerStore>()((set, get) => ({
  ...createInitialWorkspaceState(),
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

  setStatus: (status, error) =>
    set((state) => {
      const errorMessage = error ?? null;
      return state.status === status && Object.is(state.errorMessage, errorMessage)
        ? state
        : {status, errorMessage};
    }),

  setFrameSource: (source) =>
    set((state) => (state.frameSource === source ? state : {frameSource: source})),

  setConfig: (config) =>
    set((state) => (state.config === config ? state : {config})),

  incrementFrameCount: () =>
    set((state) => ({frameCount: state.frameCount + 1})),

  resetFrameCount: () =>
    set((state) => (state.frameCount === 0 ? state : {frameCount: 0})),

  addCapturedDocument: (doc) =>
    set((state) => ({
      capturedDocuments: [...state.capturedDocuments, doc],
    })),

  updateCapturedDocument: (id, updates) =>
    set((state) => {
      let didChange = false;
      const capturedDocuments = state.capturedDocuments.map((document) => {
        if (document.id !== id) return document;
        didChange = true;
        return {...document, ...updates};
      });
      return didChange ? {capturedDocuments} : state;
    }),

  removeCapturedDocument: (id) =>
    set((state) => ({
      capturedDocuments: state.capturedDocuments.filter((document) => document.id !== id),
    })),

  clearCapturedDocuments: () => set({capturedDocuments: []}),

  setPreviewDebug: (patch) =>
    set((state) => {
      const previewDebug = mergePatchIfChanged(state.previewDebug, patch, ["updatedAt"]);
      return previewDebug === state.previewDebug ? state : {previewDebug};
    }),

  setCvDebug: (patch) =>
    set((state) => {
      const cvDebug = mergePatchIfChanged(state.cvDebug, patch, ["updatedAt"]);
      return cvDebug === state.cvDebug ? state : {cvDebug};
    }),

  setConnectionDebug: (patch) =>
    set((state) => {
      const connectionDebug = mergePatchIfChanged(state.connectionDebug, patch);
      return connectionDebug === state.connectionDebug ? state : {connectionDebug};
    }),

  setCaptureDebug: (patch) =>
    set((state) => {
      const captureDebug = mergePatchIfChanged(state.captureDebug, patch);
      return captureDebug === state.captureDebug ? state : {captureDebug};
    }),

  resetDebugState: () =>
    set({
      previewDebug: createInitialPreviewDebugState(),
      cvDebug: createInitialCvDebugState(),
      connectionDebug: createInitialConnectionDebugState(),
      captureDebug: createInitialCaptureDebugState(),
    }),

  reset: () => set(createInitialWorkspaceState()),

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

const isPlainObject = (value: unknown): value is Record<string, unknown> => {
  if (!value || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
};

const isSamePatchValue = (current: unknown, next: unknown): boolean => {
  if (Object.is(current, next)) return true;

  if (Array.isArray(current) && Array.isArray(next)) {
    return current.length === next.length
      && current.every((value, index) => isSamePatchValue(value, next[index]));
  }

  if (isPlainObject(current) && isPlainObject(next)) {
    const currentKeys = Object.keys(current);
    const nextKeys = Object.keys(next);
    return currentKeys.length === nextKeys.length
      && currentKeys.every((key) => isSamePatchValue(current[key], next[key]));
  }

  return false;
};

const mergePatchIfChanged = <T extends object>(
  current: T,
  patch: Partial<T>,
  ignoredKeys: ReadonlyArray<keyof T> = [],
): T => {
  const keys = Object.keys(patch) as Array<keyof T>;
  const hasChange = keys.some((key) => {
    if (ignoredKeys.includes(key)) return false;
    return !isSamePatchValue(current[key], patch[key]);
  });
  return hasChange ? {...current, ...patch} : current;
};

const createInitialPreviewDebugState = (): ScannerPreviewDebugState => ({
  frameIndex: 0,
  previewFps: null,
  recentWindowFps: null,
  effectiveFps: null,
  payloadBytes: null,
  pollWaitMs: null,
  jsDecodeMs: null,
  canvasDrawMs: null,
  pollCount: null,
  previewWidth: null,
  previewHeight: null,
  transport: null,
  updatedAt: null,
});

const createInitialCvDebugState = (): ScannerCvDebugState => ({
  pipeline: "idle",
  cvReady: false,
  requestedBackend: "opencv",
  activeBackend: "opencv",
  strictMode: false,
  preferredProvider: null,
  preferredProviderReady: false,
  selectedModelId: null,
  selectedModelKind: null,
  selectedModelTask: null,
  backendMessage: null,
  documentDetected: false,
  cornerCount: 0,
  cornerPoints: [],
  isStable: false,
  processingWidth: null,
  processingHeight: null,
  autoCaptureEnabled: true,
  isProcessing: false,
  updatedAt: null,
});

const createInitialConnectionDebugState = (): ScannerConnectionDebugState => ({
  reconnectState: "idle",
  reconnectAttempt: null,
  reconnectMaxAttempts: null,
  reconnectDelayMs: null,
  reconnectMessage: null,
  lastErrorReason: null,
  lastDisconnectAt: null,
});

const createInitialCaptureDebugState = (): ScannerCaptureDebugState => ({
  highQualityStatus: "idle",
  highQualitySource: null,
  highQualityFallbackReason: null,
  lastCaptureSource: null,
  lastCaptureWidth: null,
  lastCaptureHeight: null,
  lastCaptureAt: null,
  lastCaptureError: null,
  lastCaptureDocumentDetected: false,
});

const createInitialWorkspaceState = () => ({
  status: "idle" as ScannerStatus,
  errorMessage: null,
  frameSource: null,
  config: null,
  frameCount: 0,
  capturedDocuments: [],
  previewDebug: createInitialPreviewDebugState(),
  cvDebug: createInitialCvDebugState(),
  connectionDebug: createInitialConnectionDebugState(),
  captureDebug: createInitialCaptureDebugState(),
});

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
