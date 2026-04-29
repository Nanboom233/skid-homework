"use client";

import {useCallback, useEffect, useRef, useState} from "react";
import type {MutableRefObject} from "react";
import {useTranslation} from "react-i18next";
import {toast} from "sonner";

import {
  createFrameSource,
  DEFAULT_SCANNER_CONFIG,
  type FrameSource,
  type FrameSourceState,
  type Point,
  type ScannerConfig,
} from "@/lib/scanner";
import {isConvexOrderedQuad, validateQuadGeometry} from "@/lib/scanner/document-quad";
import {
  type DetectionResultEvent,
  listenTauriDetectionEvents,
  probeTauriScannerDetect,
  startTauriDetectionLoop,
  stopTauriDetectionLoop,
  type TauriScannerDetectProbeResult,
} from "@/lib/tauri/scanner-detect";
import {getSelectedDesktopAdbSerial} from "@/lib/webadb/screenshot";
import {
  type ScannerConnectionDebugState,
  type ScannerCvDebugState,
  type ScannerStatus,
  useScannerStore,
} from "@/store/scanner-store";
import {
  type ScannerDetectionBackend,
  useSettingsStore,
} from "@/store/settings-store";

interface FrontendPerfSample {
  frameIndex: number;
  ipcMs: number;
  frameDecodeMs: number;
  payloadBytes: number;
  effectiveFps: number;
  pollCount: number;
}

interface DetectionBackendState {
  requestedBackend: ScannerDetectionBackend;
  activeBackend: ScannerDetectionBackend;
  ready: boolean;
  strictMode: boolean;
  message: string;
  preferredProvider: string | null;
  preferredProviderReady: boolean;
  selectedModelId: string | null;
  selectedModelKind: string | null;
  selectedModelTask: string | null;
}

type OptionalDebugFrameSource = FrameSource & {
  onDebugState?: (callback: (payload: unknown) => void) => void;
  onConnectionState?: (callback: (payload: unknown) => void) => void;
  onPerformanceState?: (callback: (payload: unknown) => void) => void;
};

export interface UseScannerSessionParams {
  isOpen: boolean;
  pushFrame: (frame: ImageData) => void;
  getLatestFrame: () => ImageData | null;
  resetPreview: () => void;
  cancelRender: () => void;
  requestAutoCapture: (frame: ImageData, points: Point[] | null) => void;
  resetCaptureRuntime: () => void;
  resetCaptureDebug: () => void;
  captureProcessingRef?: MutableRefObject<boolean>;
}

export interface UseScannerSessionResult {
  startStream: () => Promise<void>;
  stopStream: (options?: { skipComponentState?: boolean }) => Promise<void>;
  isStreaming: boolean;
  isConnecting: boolean;
  status: ScannerStatus;
  errorMessage: string | null;
  connectionState: Pick<
    ScannerConnectionDebugState,
    "reconnectState" | "reconnectAttempt" | "reconnectDelayMs" | "reconnectMessage" | "lastErrorReason"
  >;
  detectionEvents: {
    points: Point[] | null;
    isStable: boolean;
  };
  dialogOpenRef: MutableRefObject<boolean>;
  unmountedRef: MutableRefObject<boolean>;
  frameSourceRef: MutableRefObject<FrameSource | null>;
  activeDetectionBackendRef: MutableRefObject<ScannerDetectionBackend>;
  stopScannerRef: MutableRefObject<((options?: { skipComponentState?: boolean }) => Promise<void>) | null>;
}

const FRONTEND_PERF_LOG_PATTERN =
  /\[perf:frontend\]\s+frame#(\d+)\s+\|\s+ipc=([\d.]+)ms\s+frame_decode=([\d.]+)ms\s+\|\s+([\d.]+)KB\s+\|\s+effective\s+([\d.]+)\s+fps\s+\((\d+)\s+polls\)/i;
const RECOVERABLE_SIGNAL_DEDUPE_MS = 2000;

const resolveDetectionBackendState = (
  requestedBackend: ScannerDetectionBackend,
  strictMode: boolean,
  nativeSupported: boolean,
  nativeProbe: TauriScannerDetectProbeResult | null,
): DetectionBackendState => {
  const nativeReady = Boolean(
    nativeProbe?.runtimeReady
    && nativeProbe?.sessionReady
    && nativeProbe?.detectionImplemented,
  );

  if (requestedBackend === "native-ort") {
    if (nativeSupported && nativeReady) {
      return {
        requestedBackend,
        activeBackend: "native-ort",
        ready: true,
        strictMode,
        message: nativeProbe?.message
          ?? "Native runtime is active for stage-1 document detection.",
        preferredProvider: nativeProbe?.preferredProvider ?? null,
        preferredProviderReady: nativeProbe?.preferredProviderReady ?? false,
        selectedModelId: nativeProbe?.selectedModelId ?? null,
        selectedModelKind: nativeProbe?.selectedModelKind ?? null,
        selectedModelTask: nativeProbe?.selectedModelTask ?? null,
      };
    }

    if (strictMode) {
      return {
        requestedBackend,
        activeBackend: "native-ort",
        ready: false,
        strictMode,
        message: nativeProbe?.message
          ?? nativeProbe?.runtimeError
          ?? nativeProbe?.sessionError
          ?? (nativeSupported
            ? "Native runtime is not ready, and strict mode blocks OpenCV fallback."
            : "Native runtime is only available in Tauri desktop builds, and strict mode blocks OpenCV fallback."),
        preferredProvider: nativeProbe?.preferredProvider ?? null,
        preferredProviderReady: nativeProbe?.preferredProviderReady ?? false,
        selectedModelId: nativeProbe?.selectedModelId ?? null,
        selectedModelKind: nativeProbe?.selectedModelKind ?? null,
        selectedModelTask: nativeProbe?.selectedModelTask ?? null,
      };
    }

    return {
      requestedBackend,
      activeBackend: "opencv",
      ready: false,
      strictMode,
      message: nativeProbe?.message
        ?? (nativeSupported
          ? "Native runtime is not ready yet. Falling back to OpenCV."
          : "Native runtime is only available in Tauri desktop builds. Falling back to OpenCV."),
      preferredProvider: nativeProbe?.preferredProvider ?? null,
      preferredProviderReady: nativeProbe?.preferredProviderReady ?? false,
      selectedModelId: nativeProbe?.selectedModelId ?? null,
      selectedModelKind: nativeProbe?.selectedModelKind ?? null,
      selectedModelTask: nativeProbe?.selectedModelTask ?? null,
    };
  }

  return {
    requestedBackend,
    activeBackend: "opencv",
    ready: false,
    strictMode,
    message: "OpenCV contour detection is active.",
    preferredProvider: nativeProbe?.preferredProvider ?? null,
    preferredProviderReady: nativeProbe?.preferredProviderReady ?? false,
    selectedModelId: nativeProbe?.selectedModelId ?? null,
    selectedModelKind: nativeProbe?.selectedModelKind ?? null,
    selectedModelTask: nativeProbe?.selectedModelTask ?? null,
  };
};

const parseFrontendPerfLog = (value: string): FrontendPerfSample | null => {
  const match = FRONTEND_PERF_LOG_PATTERN.exec(value);
  if (!match) {
    return null;
  }

  return {
    frameIndex: Number.parseInt(match[1], 10),
    ipcMs: Number.parseFloat(match[2]),
    frameDecodeMs: Number.parseFloat(match[3]),
    payloadBytes: Math.round(Number.parseFloat(match[4]) * 1024),
    effectiveFps: Number.parseFloat(match[5]),
    pollCount: Number.parseInt(match[6], 10),
  };
};

const toFiniteNumber = (
  record: Record<string, unknown>,
  keys: string[],
): number | null => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    if (typeof value === "string" && value.length > 0) {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) {
        return parsed;
      }
    }
  }

  return null;
};

const toStringValue = (
  record: Record<string, unknown>,
  keys: string[],
): string | null => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value;
    }
  }

  return null;
};

const toReconnectState = (
  value: string | null,
):
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stopped"
  | "error"
  | null => {
  if (
    value === "idle"
    || value === "connecting"
    || value === "connected"
    || value === "reconnecting"
    || value === "stopped"
    || value === "error"
  ) {
    return value;
  }

  return null;
};

const toNullableMetric = (value: number): number | null => {
  if (!Number.isFinite(value) || value <= 0) {
    return null;
  }

  return value;
};

export function useScannerSession({
  isOpen,
  pushFrame,
  getLatestFrame,
  resetPreview,
  cancelRender,
  requestAutoCapture,
  resetCaptureRuntime,
  resetCaptureDebug,
  captureProcessingRef,
}: UseScannerSessionParams): UseScannerSessionResult {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});

  const scannerDetectionBackend = useSettingsStore((state) => state.scannerDetectionBackend);
  const scannerNativeOrtStrictMode = useSettingsStore((state) => state.scannerNativeOrtStrictMode);

  const status = useScannerStore((state) => state.status);
  const errorMessage = useScannerStore((state) => state.errorMessage);
  const connectionDebug = useScannerStore((state) => state.connectionDebug);
  const setStatus = useScannerStore((state) => state.setStatus);
  const setFrameSource = useScannerStore((state) => state.setFrameSource);
  const setConfig = useScannerStore((state) => state.setConfig);
  const setPreviewDebug = useScannerStore((state) => state.setPreviewDebug);
  const setCvDebug = useScannerStore((state) => state.setCvDebug);
  const setConnectionDebug = useScannerStore((state) => state.setConnectionDebug);
  const resetDebugState = useScannerStore((state) => state.resetDebugState);
  const reset = useScannerStore((state) => state.reset);

  const nativeBackendSupported = true;

  const frameSourceRef = useRef<FrameSource | null>(null);
  const frameSourceUnsubscribeRef = useRef<(() => void) | null>(null);
  const detectionEventUnlistenRef = useRef<(() => void) | null>(null);
  const frameSourceSessionGenerationRef = useRef(0);
  const activeDetectionBackendRef = useRef<ScannerDetectionBackend>(
    resolveDetectionBackendState(
      scannerDetectionBackend,
      scannerNativeOrtStrictMode,
      nativeBackendSupported,
      null,
    ).activeBackend,
  );
  const dialogOpenRef = useRef(isOpen);
  const unmountedRef = useRef(false);
  const lastRecoverableSignalRef = useRef<{ key: string; at: number } | null>(null);
  const lastFatalErrorToastRef = useRef<string | null>(null);
  const nativeOrtProbeRef = useRef<TauriScannerDetectProbeResult | null>(null);
  const stopScannerRef = useRef<((options?: { skipComponentState?: boolean }) => Promise<void>) | null>(null);

  const [serverJarPath, setServerJarPath] = useState<string>("");
  const [points, setPoints] = useState<Point[] | null>(null);
  const [isStable, setIsStable] = useState(false);

  const isStreaming = status === "streaming";
  const isConnecting = status === "connecting";

  const getDetectionBackendState = useCallback((): DetectionBackendState => {
    return resolveDetectionBackendState(
      scannerDetectionBackend,
      scannerNativeOrtStrictMode,
      nativeBackendSupported,
      nativeOrtProbeRef.current,
    );
  }, [
    nativeBackendSupported,
    scannerDetectionBackend,
    scannerNativeOrtStrictMode,
  ]);

  const syncDetectionBackendDebug = useCallback((patch: Partial<ScannerCvDebugState> = {}) => {
    const backendState = getDetectionBackendState();
    const nextActiveBackend = patch.activeBackend ?? backendState.activeBackend;
    activeDetectionBackendRef.current = nextActiveBackend;

    setCvDebug({
      cvReady: backendState.ready,
      requestedBackend: backendState.requestedBackend,
      activeBackend: nextActiveBackend,
      strictMode: backendState.strictMode,
      preferredProvider: backendState.preferredProvider,
      preferredProviderReady: backendState.preferredProviderReady,
      selectedModelId: backendState.selectedModelId,
      selectedModelKind: backendState.selectedModelKind,
      selectedModelTask: backendState.selectedModelTask,
      backendMessage: backendState.message,
      ...patch,
      updatedAt: Date.now(),
    });
  }, [getDetectionBackendState, setCvDebug]);

  const applyNativeORTProbe = useCallback((probe: TauriScannerDetectProbeResult | null) => {
    nativeOrtProbeRef.current = probe;
    syncDetectionBackendDebug();
  }, [syncDetectionBackendDebug]);

  const clearNativeORTProbe = useCallback((message?: string) => {
    nativeOrtProbeRef.current = null;
    syncDetectionBackendDebug(
      message
        ? {
          backendMessage: message,
        }
        : {},
    );
  }, [syncDetectionBackendDebug]);

  const refreshNativeOrtProbe = useCallback(async (): Promise<TauriScannerDetectProbeResult | null> => {
    if (!nativeBackendSupported) {
      return null;
    }

    try {
      return await probeTauriScannerDetect();
    } catch (error) {
      console.warn("[Scanner] Native ORT probe failed:", error);
      return null;
    }
  }, [nativeBackendSupported]);

  useEffect(() => {
    if (!isOpen) {
      clearNativeORTProbe();
      return;
    }

    let cancelled = false;

    void (async () => {
      const probe = await refreshNativeOrtProbe();
      if (cancelled) return;

      if (probe) {
        applyNativeORTProbe(probe);
      } else {
        clearNativeORTProbe();
        if (scannerDetectionBackend === "native-ort" && scannerNativeOrtStrictMode) {
          syncDetectionBackendDebug();
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    applyNativeORTProbe,
    clearNativeORTProbe,
    isOpen,
    refreshNativeOrtProbe,
    scannerDetectionBackend,
    scannerNativeOrtStrictMode,
    syncDetectionBackendDebug,
  ]);

  const clearFrameSourceSubscription = useCallback(() => {
    if (frameSourceUnsubscribeRef.current) {
      frameSourceUnsubscribeRef.current();
      frameSourceUnsubscribeRef.current = null;
    }
  }, []);

  const beginFrameSourceSession = useCallback((): number => {
    frameSourceSessionGenerationRef.current += 1;
    return frameSourceSessionGenerationRef.current;
  }, []);

  const isFrameSourceSessionCurrent = useCallback((
    generation: number,
    source?: FrameSource | null,
  ): boolean => {
    if (generation !== frameSourceSessionGenerationRef.current) {
      return false;
    }

    if (typeof source !== "undefined" && frameSourceRef.current !== source) {
      return false;
    }

    return true;
  }, []);

  const releaseFrameSourceIfCurrent = useCallback((source: FrameSource | null): void => {
    if (!source || frameSourceRef.current !== source) {
      return;
    }

    clearFrameSourceSubscription();
    frameSourceRef.current = null;
    setFrameSource(null);
  }, [clearFrameSourceSubscription, setFrameSource]);

  const applyPerfSample = useCallback((sample: FrontendPerfSample) => {
    setPreviewDebug({
      frameIndex: sample.frameIndex,
      payloadBytes: sample.payloadBytes,
      effectiveFps: sample.effectiveFps,
      pollCount: sample.pollCount,
      updatedAt: Date.now(),
    });
  }, [setPreviewDebug]);

  const applyRecoverableScannerSignal = useCallback((message: string, origin: string) => {
    const normalizedMessage = message.trim();
    if (normalizedMessage.length === 0) {
      return;
    }

    const now = Date.now();
    const signature = `${origin}:${normalizedMessage}`;
    const previousSignal = lastRecoverableSignalRef.current;

    if (
      previousSignal
      && previousSignal.key === signature
      && now - previousSignal.at < RECOVERABLE_SIGNAL_DEDUPE_MS
    ) {
      return;
    }

    lastRecoverableSignalRef.current = {
      key: signature,
      at: now,
    };

    setConnectionDebug({
      reconnectState: "reconnecting",
      reconnectMessage: t("connection.recovering"),
      lastErrorReason: normalizedMessage,
      lastDisconnectAt: now,
    });
    setStatus("connecting");
  }, [setConnectionDebug, setStatus, t]);

  const applyFrameSourceState = useCallback((state: FrameSourceState) => {
    const nextReconnectState = (() => {
      switch (state.status) {
        case "starting":
          return "connecting";
        case "streaming":
          return "connected";
        case "reconnecting":
          return "reconnecting";
        case "stopping":
        case "stopped":
          return "stopped";
        case "error":
          return "error";
        default:
          return "idle";
      }
    })();

    const reconnectMessage = (() => {
      switch (state.status) {
        case "starting":
          return t("connection.starting");
        case "streaming":
          return state.reconnectAttempt > 0
            ? t("connection.recovered")
            : t("connection.active");
        case "reconnecting":
          return state.nextReconnectDelayMs !== null
            ? t("connection.recovering-delay", {delayMs: state.nextReconnectDelayMs})
            : t("connection.recovering");
        case "stopping":
        case "stopped":
          return t("connection.stopped");
        case "error":
          return state.lastError ?? t("connection.failed");
        default:
          return t("connection.idle");
      }
    })();

    setPreviewDebug({
      frameIndex: state.metrics.frameIndex,
      previewFps: toNullableMetric(state.metrics.previewFps),
      recentWindowFps: toNullableMetric(state.metrics.recentWindowFps),
      effectiveFps: toNullableMetric(state.metrics.effectiveFps),
      payloadBytes: toNullableMetric(state.metrics.lastPayloadBytes),
      pollWaitMs: toNullableMetric(state.metrics.lastIpcMs),
      jsDecodeMs: toNullableMetric(state.metrics.lastDecodeMs),
      pollCount: state.metrics.pollCount > 0 ? state.metrics.pollCount : null,
      previewWidth: state.metrics.previewWidth,
      previewHeight: state.metrics.previewHeight,
      updatedAt: Date.now(),
    });

    setConnectionDebug({
      reconnectState: nextReconnectState,
      reconnectAttempt: state.reconnectAttempt > 0 ? state.reconnectAttempt : null,
      reconnectDelayMs: state.nextReconnectDelayMs,
      reconnectMessage,
      lastErrorReason: state.lastError,
      lastDisconnectAt:
        state.status === "reconnecting" || state.status === "error" || state.status === "stopped"
          ? Date.now()
          : null,
    });

    if (state.status === "streaming") {
      lastFatalErrorToastRef.current = null;
      setStatus("streaming");
      return;
    }

    if (state.status === "starting" || state.status === "reconnecting") {
      setStatus("connecting");
      return;
    }

    if (state.status === "error") {
      const fatalMessage = state.lastError ?? state.stopReason ?? t("connection.failed");
      setStatus("error", fatalMessage);
      if (lastFatalErrorToastRef.current !== fatalMessage) {
        lastFatalErrorToastRef.current = fatalMessage;
        toast.error(t("toasts.scanner-error", {message: fatalMessage}));
      }
      return;
    }

    if (state.status === "stopping" || state.status === "stopped") {
      setStatus("idle");
      return;
    }

    setStatus("idle");
  }, [setConnectionDebug, setPreviewDebug, setStatus, t]);

  const applyFutureDebugPayload = useCallback((payload: unknown) => {
    if (typeof payload === "string") {
      const perf = parseFrontendPerfLog(payload);
      if (perf) {
        applyPerfSample(perf);
        return;
      }

      setConnectionDebug({
        reconnectMessage: payload,
      });
      return;
    }

    if (!payload || typeof payload !== "object") {
      return;
    }

    const record = payload as Record<string, unknown>;
    const previewPatch: Parameters<typeof setPreviewDebug>[0] = {};
    const connectionPatch: Parameters<typeof setConnectionDebug>[0] = {};

    const frameIndex = toFiniteNumber(record, ["frameIndex", "frame", "frameCount"]);
    const previewWidth = toFiniteNumber(record, ["previewWidth", "width"]);
    const previewHeight = toFiniteNumber(record, ["previewHeight", "height"]);
    const previewFps = toFiniteNumber(record, ["previewFps", "currentFps"]);
    const recentWindowFps = toFiniteNumber(record, ["recentWindowFps", "windowFps"]);
    const effectiveFps = toFiniteNumber(record, ["effectiveFps"]);
    const frameDecodeMs = toFiniteNumber(record, ["frameDecodeMs", "decodeMs"]);
    const payloadBytes = toFiniteNumber(record, ["payloadBytes", "payloadSize"]);
    const payloadKb = toFiniteNumber(record, ["payloadKb"]);
    const pollCount = toFiniteNumber(record, ["pollCount", "polls"]);
    const transport = toStringValue(record, ["transport", "codec", "pipeline"]);

    if (frameIndex !== null) previewPatch.frameIndex = Math.round(frameIndex);
    if (previewWidth !== null) previewPatch.previewWidth = Math.round(previewWidth);
    if (previewHeight !== null) previewPatch.previewHeight = Math.round(previewHeight);
    if (previewFps !== null) previewPatch.previewFps = previewFps;
    if (recentWindowFps !== null) previewPatch.recentWindowFps = recentWindowFps;
    if (effectiveFps !== null) previewPatch.effectiveFps = effectiveFps;
    if (frameDecodeMs !== null) previewPatch.jsDecodeMs = frameDecodeMs;
    if (payloadBytes !== null) previewPatch.payloadBytes = payloadBytes;
    if (payloadKb !== null) previewPatch.payloadBytes = Math.round(payloadKb * 1024);
    if (pollCount !== null) previewPatch.pollCount = Math.round(pollCount);
    if (transport !== null) previewPatch.transport = transport;

    const reconnectState = toReconnectState(
      toStringValue(record, ["reconnectState", "connectionState", "state"]),
    );
    if (reconnectState !== null) connectionPatch.reconnectState = reconnectState;

    const reconnectAttempt = toFiniteNumber(record, ["reconnectAttempt", "attempt"]);
    const reconnectMaxAttempts = toFiniteNumber(record, [
      "reconnectMaxAttempts",
      "maxAttempts",
    ]);
    const reconnectDelayMs = toFiniteNumber(record, ["reconnectDelayMs", "delayMs"]);
    const reconnectMessage = toStringValue(record, ["message", "reconnectMessage"]);
    const lastErrorReason = toStringValue(record, ["lastErrorReason", "error"]);

    if (reconnectAttempt !== null) {
      connectionPatch.reconnectAttempt = Math.round(reconnectAttempt);
    }
    if (reconnectMaxAttempts !== null) {
      connectionPatch.reconnectMaxAttempts = Math.round(reconnectMaxAttempts);
    }
    if (reconnectDelayMs !== null) connectionPatch.reconnectDelayMs = reconnectDelayMs;
    if (reconnectMessage !== null) connectionPatch.reconnectMessage = reconnectMessage;
    if (lastErrorReason !== null) connectionPatch.lastErrorReason = lastErrorReason;

    if (Object.keys(previewPatch).length > 0) {
      setPreviewDebug({...previewPatch, updatedAt: Date.now()});
    }
    if (Object.keys(connectionPatch).length > 0) {
      setConnectionDebug(connectionPatch);
    }
  }, [applyPerfSample, setConnectionDebug, setPreviewDebug]);

  const attachOptionalHooks = useCallback((
    source: FrameSource,
    shouldApplyPayload?: () => boolean,
  ) => {
    const maybeDebugSource = source as OptionalDebugFrameSource;
    const applyGuardedPayload = (payload: unknown): void => {
      if (shouldApplyPayload && !shouldApplyPayload()) {
        return;
      }

      applyFutureDebugPayload(payload);
    };

    maybeDebugSource.onDebugState?.(applyGuardedPayload);
    maybeDebugSource.onConnectionState?.(applyGuardedPayload);
    maybeDebugSource.onPerformanceState?.(applyGuardedPayload);
  }, [applyFutureDebugPayload]);

  const startStream = useCallback(async () => {
    if (!dialogOpenRef.current || unmountedRef.current) {
      return;
    }

    const serial = getSelectedDesktopAdbSerial();
    if (!serial) {
      toast.error(t("toasts.no-device"));
      return;
    }

    const sessionGeneration = beginFrameSourceSession();
    const isCurrentStartSession = (source?: FrameSource | null): boolean => {
      return isFrameSourceSessionCurrent(sessionGeneration, source);
    };

    let resolvedJarPath = serverJarPath;
    if (!resolvedJarPath) {
      try {
        const {resolveResource} = await import("@tauri-apps/api/path");
        resolvedJarPath = await resolveResource("resources/camera-server.jar");
        if (
          !dialogOpenRef.current
          || !isCurrentStartSession()
        ) {
          return;
        }
        setServerJarPath(resolvedJarPath);
      } catch {
        if (
          dialogOpenRef.current
          && isCurrentStartSession()
        ) {
          toast.error(t("toasts.server-jar-missing"));
        }
        return;
      }
    }

    if (
      !dialogOpenRef.current
      || !isCurrentStartSession()
    ) {
      return;
    }

    const config: ScannerConfig = {
      ...DEFAULT_SCANNER_CONFIG,
      serial,
      serverJarPath: resolvedJarPath,
    };

    cancelRender();
    resetPreview();
    setStatus("connecting");
    setConfig(config);
    resetDebugState();
    syncDetectionBackendDebug();
    setPoints(null);
    setIsStable(false);

    setPreviewDebug({
      transport: "live-preview",
      previewWidth: config.width,
      previewHeight: config.height,
      updatedAt: Date.now(),
    });
    setConnectionDebug({
      reconnectState: "connecting",
      reconnectMessage: t("connection.starting"),
      lastErrorReason: null,
      reconnectAttempt: null,
      reconnectMaxAttempts: null,
      reconnectDelayMs: null,
    });

    let source: FrameSource | null = null;

    try {
      source = createFrameSource(config);

      const isCurrentSourceSession = (): boolean => {
        return isCurrentStartSession(source);
      };

      attachOptionalHooks(source, isCurrentSourceSession);
      clearFrameSourceSubscription();
      frameSourceUnsubscribeRef.current = source.onStateChange((state: FrameSourceState) => {
        if (!isCurrentSourceSession()) {
          return;
        }

        applyFrameSourceState(state);
      });

      source.onFrame((frame: ImageData) => {
        if (!isCurrentSourceSession()) {
          return;
        }

        pushFrame(frame);
      });

      source.onError((error: string) => {
        if (!isCurrentSourceSession()) {
          return;
        }

        console.error("[Scanner] Frame source error:", error);
        applyRecoverableScannerSignal(error, "source.onError");
      });

      frameSourceRef.current = source;
      setFrameSource(source);

      if (
        !dialogOpenRef.current
        || !isCurrentSourceSession()
      ) {
        releaseFrameSourceIfCurrent(source);
        try {
          await source.stop();
        } catch (error) {
          console.warn("[Scanner] Failed to stop frame source:", error);
        }
        return;
      }

      await source.start();

      if (
        !dialogOpenRef.current
        || !isCurrentSourceSession()
      ) {
        releaseFrameSourceIfCurrent(source);
        try {
          await source.stop();
        } catch (error) {
          console.warn("[Scanner] Failed to stop frame source:", error);
        }
        return;
      }

      setStatus("streaming");
      setConnectionDebug({
        reconnectState: "connected",
        reconnectMessage: t("connection.started"),
      });

      const detectionBackend = activeDetectionBackendRef.current ?? "opencv";

      try {
        const detectionUnlisten = await listenTauriDetectionEvents((event: DetectionResultEvent) => {
          if (captureProcessingRef?.current) {
            return;
          }

          if (!isCurrentSourceSession()) {
            return;
          }

          const detectedPoints = event.effectivePoints ?? event.points ?? null;
          const acceptedPoints =
            detectedPoints && detectedPoints.length === 4
            && isConvexOrderedQuad(detectedPoints)
            && validateQuadGeometry(detectedPoints, event.frameWidth, event.frameHeight).valid
              ? detectedPoints
              : null;
          setPoints(acceptedPoints);
          setIsStable(acceptedPoints ? event.isStable : false);

          if (!event.autoCaptureTriggered) {
            return;
          }

          const frame = getLatestFrame();
          if (frame) {
            requestAutoCapture(frame, acceptedPoints);
          }
        });

        if (!dialogOpenRef.current || !isCurrentSourceSession()) {
          detectionUnlisten();
          return;
        }

        detectionEventUnlistenRef.current = detectionUnlisten;

        await startTauriDetectionLoop({backend: detectionBackend});

        if (!dialogOpenRef.current || !isCurrentSourceSession()) {
          if (detectionEventUnlistenRef.current === detectionUnlisten) {
            detectionEventUnlistenRef.current = null;
          }
          detectionUnlisten();
          await stopTauriDetectionLoop().catch((e) =>
            console.warn("[Scanner] Stale detection loop cleanup:", e),
          );
          return;
        }
      } catch (detectionError) {
        console.warn("[Scanner] Failed to start detection loop:", detectionError);
        setCvDebug({
          backendMessage: `Detection unavailable: ${detectionError instanceof Error ? detectionError.message : String(detectionError)}`,
        });
      }
    } catch (error) {
      if (!source || !isCurrentStartSession(source)) {
        releaseFrameSourceIfCurrent(source);
        return;
      }

      releaseFrameSourceIfCurrent(source);
      const message = error instanceof Error ? error.message : String(error);
      setStatus("error", message);
      setConnectionDebug({
        reconnectState: "error",
        reconnectMessage: t("connection.start-failed"),
        lastErrorReason: message,
        lastDisconnectAt: Date.now(),
      });
      toast.error(t("toasts.start-failed", {message}));
    }
  }, [
    applyFrameSourceState,
    applyRecoverableScannerSignal,
    attachOptionalHooks,
    beginFrameSourceSession,
    cancelRender,
    captureProcessingRef,
    clearFrameSourceSubscription,
    getLatestFrame,
    isFrameSourceSessionCurrent,
    pushFrame,
    releaseFrameSourceIfCurrent,
    requestAutoCapture,
    resetDebugState,
    resetPreview,
    serverJarPath,
    setCvDebug,
    setConfig,
    setConnectionDebug,
    setFrameSource,
    setPreviewDebug,
    setStatus,
    syncDetectionBackendDebug,
    t,
  ]);

  const stopStream = useCallback(async (options?: { skipComponentState?: boolean }) => {
    const skipComponentState = options?.skipComponentState ?? false;

    const sessionGeneration = beginFrameSourceSession();
    const source = frameSourceRef.current;
    let stopErrorMessage: string | null = null;

    if (!skipComponentState && !unmountedRef.current) {
      resetCaptureRuntime();
    }
    clearFrameSourceSubscription();

    if (detectionEventUnlistenRef.current) {
      detectionEventUnlistenRef.current();
      detectionEventUnlistenRef.current = null;
    }
    await stopTauriDetectionLoop().catch((e) =>
      console.warn("[Scanner] Failed to stop detection loop:", e),
    );

    cancelRender();

    if (source) {
      frameSourceRef.current = null;
      setFrameSource(null);
      try {
        await source.stop();
      } catch (error) {
        stopErrorMessage = error instanceof Error ? error.message : String(error);
        console.warn("[Scanner] Failed to stop frame source:", error);
      }
    }

    if (!isFrameSourceSessionCurrent(sessionGeneration)) {
      return;
    }

    if (!skipComponentState && !unmountedRef.current) {
      resetPreview();
      resetCaptureDebug();
      setPoints(null);
      setIsStable(false);
    }

    setConfig(null);
    if (stopErrorMessage) {
      setConnectionDebug({
        reconnectState: "error",
        reconnectMessage: stopErrorMessage,
        lastErrorReason: stopErrorMessage,
        lastDisconnectAt: Date.now(),
      });
      setStatus("error", stopErrorMessage);
      return;
    }

    setConnectionDebug({
      reconnectState: "stopped",
      reconnectMessage: t("connection.stopped-by-user"),
      lastErrorReason: null,
      lastDisconnectAt: Date.now(),
    });
    setStatus("idle");
  }, [
    beginFrameSourceSession,
    cancelRender,
    clearFrameSourceSubscription,
    isFrameSourceSessionCurrent,
    resetCaptureDebug,
    resetCaptureRuntime,
    resetPreview,
    setConfig,
    setConnectionDebug,
    setFrameSource,
    setStatus,
    t,
  ]);

  // Keep a stable ref so the unmount-only cleanup always calls the latest version.
  stopScannerRef.current = stopStream;

  useEffect(() => {
    const wasOpen = dialogOpenRef.current;
    dialogOpenRef.current = isOpen;
    if (wasOpen && !isOpen) {
      void stopScannerRef.current?.();
      reset();
    }
  }, [isOpen, reset]);

  useEffect(() => {
    // Strict Mode mounts effects twice in development, so reset the ref here
    // after the previous cleanup marks the component as unmounted.
    unmountedRef.current = false;

    return () => {
      unmountedRef.current = true;
      dialogOpenRef.current = false;
      void stopScannerRef.current?.({skipComponentState: true});
    };
  }, []);

  return {
    startStream,
    stopStream,
    isStreaming,
    isConnecting,
    status,
    errorMessage,
    connectionState: {
      reconnectState: connectionDebug.reconnectState,
      reconnectAttempt: connectionDebug.reconnectAttempt,
      reconnectDelayMs: connectionDebug.reconnectDelayMs,
      reconnectMessage: connectionDebug.reconnectMessage,
      lastErrorReason: connectionDebug.lastErrorReason,
    },
    detectionEvents: {
      points,
      isStable,
    },
    dialogOpenRef,
    unmountedRef,
    frameSourceRef,
    activeDetectionBackendRef,
    stopScannerRef,
  };
}