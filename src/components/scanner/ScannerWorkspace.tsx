import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import type {KeyboardEvent as ReactKeyboardEvent} from "react";
import {useTranslation} from "react-i18next";

import {useScannerCapture, type UseScannerCaptureResult} from "@/hooks/use-scanner-capture";
import {useScannerPreview} from "@/hooks/use-scanner-preview";
import {useScannerSession} from "@/hooks/use-scanner-session";
import {useMediaQuery} from "@/hooks/use-media-query";
import {useSettingsStore} from "@/store/settings-store";
import {Dialog, DialogContent, DialogTitle} from "@/components/ui/dialog";
import {CapturedDocumentTray} from "./CapturedDocumentTray";
import {ScannerActionBar} from "./ScannerActionBar";
import {ScannerCapturedDocumentEditor} from "./ScannerCapturedDocumentEditor";
import {ScannerControls} from "./ScannerControls";
import {ScannerDiagnosticsDrawer} from "./ScannerDiagnosticsDrawer";
import {ScannerHeader} from "./ScannerHeader";
import {ScannerOverlay} from "./ScannerOverlay";
import {ScannerPreviewHud} from "./ScannerPreviewHud";
import {Loader2} from "lucide-react";

interface ScannerWorkspaceProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onDocumentsCaptured: (files: File[]) => void;
}

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "area[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "iframe",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex='-1'])",
].join(", ");

const isElementVisible = (element: HTMLElement): boolean => {
  // offsetParent is null for hidden/display:none elements, much faster than getComputedStyle
  return element.offsetParent !== null || element === document.body;
};

const getFocusableElements = (container: HTMLElement): HTMLElement[] => {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter((element) => {
    return !element.hasAttribute("disabled")
      && element.tabIndex >= 0
      && isElementVisible(element);
  });
};

const isInteractiveTarget = (target: EventTarget | null): boolean => {
  const element = target instanceof HTMLElement ? target : null;
  if (!element) {
    return false;
  }

  if (element.isContentEditable) {
    return true;
  }

  return Boolean(
    element.closest(
      "button, [role='button'], a[href], input, textarea, select, summary, [contenteditable='true']",
    ),
  );
};

export function ScannerWorkspace({
  isOpen,
  onOpenChange,
  onDocumentsCaptured,
}: ScannerWorkspaceProps) {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});
  const prefersReducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)");

  const scannerPostProcessBackend = useSettingsStore((s) => s.scannerPostProcessBackend);
  const setScannerPostProcessBackend = useSettingsStore((s) => s.setScannerPostProcessBackend);

  const [editingDocId, setEditingDocId] = useState<string | null>(null);
  const [isDiagnosticsOpen, setIsDiagnosticsOpen] = useState(false);
  const [captureFlash, setCaptureFlash] = useState(false);

  const workspaceRef = useRef<HTMLDivElement | null>(null);
  const captureFlashTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const captureProcessingRef = useRef(false);

  // TODO: Stable ref to break circular dependency: session needs capture bridges,
  // but capture needs session's runtime refs. The ref is populated after
  // useScannerCapture returns, and only read asynchronously (never during render).
  // This is acceptable because the refs are stable and only accessed during event handlers.
  const captureRef = useRef<UseScannerCaptureResult | null>(null);

  // --- Hook composition (order matters: preview → session → capture) ---

  const preview = useScannerPreview();

  const requestAutoCapture = useCallback(
    (...args: Parameters<NonNullable<UseScannerCaptureResult["requestAutoCapture"]>>) =>
      captureRef.current?.requestAutoCapture(...args),
    [],
  );
  const resetCaptureRuntime = useCallback(() => captureRef.current?.resetCaptureRuntime(), []);
  const resetCaptureDebug = useCallback(() => captureRef.current?.resetCaptureDebug(), []);

  const session = useScannerSession({
    isOpen,
    pushFrame: preview.pushFrame,
    getLatestFrame: preview.getLatestFrame,
    resetPreview: preview.resetPreview,
    cancelRender: preview.cancelRender,
    requestAutoCapture,
    resetCaptureRuntime,
    resetCaptureDebug,
    captureProcessingRef,
  });

  const capture = useScannerCapture({
    dialogOpenRef: session.dialogOpenRef,
    frameSourceRef: session.frameSourceRef,
    activeDetectionBackendRef: session.activeDetectionBackendRef,
    getLatestFrame: preview.getLatestFrame,
    captureProcessingRef,
    orientationRef: preview.orientationRef,
  });

  // Sync capture ref after hooks are established (only read asynchronously).
  useEffect(() => {
    captureRef.current = capture;
  });

  const {canvasRef, orientation, setOrientation, previewDimensions} = preview;
  const {
    startStream,
    stopStream,
    isStreaming,
    isConnecting,
    detectionEvents,
  } = session;

  const editingDocument = useMemo(
    () => capture.capturedDocuments.find((d) => d.id === editingDocId) ?? null,
    [capture.capturedDocuments, editingDocId],
  );

  // M1 fix: derive isEditing from actual document existence, not just ID.
  const isEditing = editingDocument !== null;

  const deviceStatus = isStreaming
    ? "connected" as const
    : isConnecting
      ? "connecting" as const
      : session.status === "error"
        ? "error" as const
        : "disconnected" as const;

  const handleExitEditor = useCallback(() => {
    setEditingDocId(null);
  }, []);

  const handleToggleDiagnostics = useCallback(() => {
    setIsDiagnosticsOpen((current) => !current);
  }, []);

  const clearCaptureFlashTimeout = useCallback(() => {
    if (captureFlashTimeoutRef.current) {
      clearTimeout(captureFlashTimeoutRef.current);
      captureFlashTimeoutRef.current = null;
    }
  }, []);

  const triggerCaptureFlash = useCallback(() => {
    if (prefersReducedMotion) return;
    clearCaptureFlashTimeout();
    setCaptureFlash(true);
    captureFlashTimeoutRef.current = setTimeout(() => {
      setCaptureFlash(false);
      captureFlashTimeoutRef.current = null;
    }, 150);
  }, [clearCaptureFlashTimeout, prefersReducedMotion]);

  useEffect(() => {
    return clearCaptureFlashTimeout;
  }, [clearCaptureFlashTimeout]);

  useEffect(() => {
    if (!isOpen) {
      clearCaptureFlashTimeout();
    }
  }, [clearCaptureFlashTimeout, isOpen]);

  const handleCloseWorkspace = useCallback(() => {
    // M1 fix: Reset local UI state to prevent stale editing state on reopen.
    setEditingDocId(null);
    setIsDiagnosticsOpen(false);
    onOpenChange(false);
  }, [onOpenChange]);

  const handleCapture = useCallback(() => {
    if (!isStreaming || isEditing || capture.isCapturing) {
      return;
    }

    triggerCaptureFlash();
    void capture.capture();
  }, [capture, isEditing, isStreaming, triggerCaptureFlash]);

  const handleToggleOrientation = useCallback(() => {
    if (capture.isCapturing) return;
    setOrientation((prev) => (prev === "landscape" ? "portrait" : "landscape"));
  }, [capture.isCapturing, setOrientation]);

  const handleSendToAI = useCallback(() => {
    // M2 fix: block send while a capture is still being committed.
    if (capture.isCaptureCommitPending) {
      return;
    }

    const readyFiles = capture.capturedDocuments
      .filter((doc) => doc.status === "ready")
      .map((doc) => doc.file);

    if (readyFiles.length > 0) {
      onDocumentsCaptured(readyFiles);
      handleCloseWorkspace();
    }
  }, [capture.capturedDocuments, capture.isCaptureCommitPending, handleCloseWorkspace, onDocumentsCaptured]);

  const handleStartStop = useCallback(() => {
    if (isStreaming) {
      void stopStream();
    } else {
      void startStream();
    }
  }, [isStreaming, startStream, stopStream]);

  const handleDocumentKeyDown = useCallback((event: KeyboardEvent) => {
    if (!isOpen) {
      return;
    }

    if (event.code === "Space") {
      if (event.repeat || isInteractiveTarget(event.target) || !isStreaming || isEditing) {
        return;
      }

      event.preventDefault();
      handleCapture();
      return;
    }

    if (event.key === "Escape") {
      if (event.defaultPrevented) return;
      if (isEditing || isDiagnosticsOpen) return;
      event.preventDefault();
      handleCloseWorkspace();
      return;
    }

    if (
      (event.ctrlKey || event.metaKey)
      && event.shiftKey
      && !event.altKey
      && event.key.toLowerCase() === "d"
    ) {
      event.preventDefault();
      handleToggleDiagnostics();
    }
  }, [
    handleCapture,
    handleCloseWorkspace,
    handleToggleDiagnostics,
    isDiagnosticsOpen,
    isEditing,
    isOpen,
    isStreaming,
  ]);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    document.addEventListener("keydown", handleDocumentKeyDown);

    return () => {
      document.removeEventListener("keydown", handleDocumentKeyDown);
    };
  }, [handleDocumentKeyDown, isOpen]);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    const frameId = requestAnimationFrame(() => {
      workspaceRef.current?.focus({preventScroll: true});
    });

    return () => {
      cancelAnimationFrame(frameId);
    };
  }, [isOpen]);

  const handleWorkspaceKeyDownCapture = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") {
      return;
    }

    const workspaceElement = workspaceRef.current;
    if (!workspaceElement) {
      return;
    }

    const focusableElements = getFocusableElements(workspaceElement);
    if (focusableElements.length === 0) {
      event.preventDefault();
      workspaceElement.focus();
      return;
    }

    const activeElement = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    const firstElement = focusableElements[0];
    const lastElement = focusableElements[focusableElements.length - 1];

    if (event.shiftKey) {
      if (activeElement === workspaceElement || activeElement === firstElement) {
        event.preventDefault();
        lastElement.focus();
      }
      return;
    }

    if (activeElement === workspaceElement) {
      event.preventDefault();
      firstElement.focus();
      return;
    }

    if (activeElement === lastElement) {
      event.preventDefault();
      firstElement.focus();
    }
  }, []);

  const {capturedDocuments} = capture;
  const hasProcessingDocs = capturedDocuments.some((d) => d.status === "processing");
  const hasFailedDocs = capturedDocuments.some((d) => d.status === "failed");

  const hasFrameData = previewDimensions.displayWidth > 0 && previewDimensions.displayHeight > 0;
  const aspectValue = hasFrameData
    ? previewDimensions.displayWidth / previewDimensions.displayHeight
    : (orientation === "portrait" ? 3 / 4 : 16 / 9);

  const hasTray = !isEditing && capturedDocuments.length > 0;
  const sidePanelsWidth = isEditing ? "0px" : hasTray ? "44rem" : "24rem";

  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent
        size="scanner"
        className="!flex h-[min(85vh,820px)] flex-col overflow-hidden p-0 transition-[max-width] duration-300 ease-out"
        showCloseButton={false}
        style={{
          "--camera-aspect": aspectValue,
          "--side-panels-w": sidePanelsWidth,
          "--preview-h": "calc(min(85vh, 820px) - 56px - 32px)",
          "--ideal-w": "calc((var(--preview-h) * var(--camera-aspect)) + var(--side-panels-w))",
          width: "100%",
          maxWidth: "min(calc(100vw - 2rem), var(--ideal-w), 1280px)",
        } as React.CSSProperties}
      >
        <DialogTitle className="sr-only">{t("title", "Scanner")}</DialogTitle>
        <div
          ref={workspaceRef}
          tabIndex={-1}
          onKeyDownCapture={handleWorkspaceKeyDownCapture}
          className="flex h-full w-full flex-col outline-none"
        >
      <ScannerHeader
        onClose={handleCloseWorkspace}
        isEditing={isEditing}
        editingLabel={isEditing ? t("editor.title", "Edit Document") : undefined}
        deviceStatus={deviceStatus}
        onToggleOrientation={!isEditing ? handleToggleOrientation : undefined}
        onToggleDiagnostics={handleToggleDiagnostics}
      />

      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        {/* Preview Stage */}
        <main
          className="flex min-h-0 flex-1 flex-col items-center justify-center overflow-hidden bg-black p-4"
          style={{ containerType: "size" } as React.CSSProperties}
        >
          <div
            className="relative self-center overflow-hidden rounded-xl border bg-black shadow-inner"
            style={{
              aspectRatio:
                previewDimensions.displayWidth > 0 && previewDimensions.displayHeight > 0
                  ? `${previewDimensions.displayWidth} / ${previewDimensions.displayHeight}`
                  : orientation === "portrait" ? "3 / 4" : "16 / 9",
              width: `min(100cqw, 100cqh * ${
                previewDimensions.displayWidth > 0 && previewDimensions.displayHeight > 0
                  ? previewDimensions.displayWidth / previewDimensions.displayHeight
                  : orientation === "portrait" ? 3 / 4 : 16 / 9
              })`,
              maxWidth: "100%",
              maxHeight: "100%",
            }}
          >
            <canvas
              ref={canvasRef}
              className="h-full w-full object-contain"
              role="img"
              aria-label="Scanner preview"
            />

            {isStreaming && (
              <>
                <ScannerOverlay
                  points={detectionEvents.points}
                  isStable={detectionEvents.isStable}
                  frameWidth={previewDimensions.frameWidth}
                  frameHeight={previewDimensions.frameHeight}
                  orientation={orientation}
                />
                <ScannerPreviewHud />
              </>
            )}

            <div
              aria-hidden="true"
              className={`absolute inset-0 bg-white pointer-events-none transition-opacity duration-150 ${
                captureFlash ? "opacity-100" : "opacity-0"
              }`}
            />

            {capture.isCapturing && (
              <div className="absolute inset-0 z-10 bg-black/35 pointer-events-none flex items-center justify-center">
                <Loader2 className="h-8 w-8 animate-spin text-white/80" />
              </div>
            )}
          </div>

          {/* Mobile Action Bar */}
          <div className="w-full shrink-0 border-t bg-background/95 backdrop-blur lg:hidden">
            <ScannerActionBar
              onCapture={handleCapture}
              isCapturing={capture.isCapturing}
              autoCapture={capture.autoCapture}
              onAutoCaptureChange={capture.setAutoCapture}
              isStreaming={isStreaming}
              onStartStop={handleStartStop}
              isConnecting={isConnecting}
              disabled={isEditing}
            />
          </div>
        </main>

        {/* Desktop Controls (Vertical) */}
        {!isEditing && (
          <div className="hidden w-96 shrink-0 flex-col overflow-y-auto border-l bg-background p-4 lg:flex">
            <ScannerControls
              isConnecting={isConnecting}
              isStreaming={isStreaming}
              isProcessing={capture.isCapturing || hasProcessingDocs}
              autoCapture={capture.autoCapture}
              isStable={detectionEvents.isStable}
              requestedPostProcessBackend={scannerPostProcessBackend}
              previewOrientation={orientation}
              onPostProcessBackendChange={setScannerPostProcessBackend}
              onAutoCaptureChange={capture.setAutoCapture}
              onPreviewOrientationToggle={handleToggleOrientation}
              onStart={session.startStream}
              onStop={session.stopStream}
              onPreviewCapture={handleCapture}
            />
          </div>
        )}

        {/* Document Tray */}
        {!isEditing && (
          <aside className="shrink-0 border-t bg-muted/20 lg:border-l lg:border-t-0">
            <CapturedDocumentTray
              documents={capturedDocuments}
              onEdit={setEditingDocId}
              onRemove={capture.removeCapturedDocument}
              onSendToAI={handleSendToAI}
              sendDisabled={capturedDocuments.length === 0 || hasProcessingDocs || hasFailedDocs || capture.isCaptureCommitPending}
              sendLabel={t("actions.send-to-ai", {count: capturedDocuments.length})}
            />
          </aside>
        )}
      </div>

      {/* Editor */}
      <ScannerCapturedDocumentEditor
        open={isEditing}
        document={editingDocument}
        isApplying={editingDocument?.status === "processing"}
        onOpenChange={(open) => {
          if (!open) {
            handleExitEditor();
          }
        }}
        onApply={(id, points, options) => {
          capture.reprocessCapturedDocument(id, points, options);
          handleExitEditor();
        }}
        onPreviewRequest={capture.getEditorPreview}
      />

      {/* Diagnostics */}
      <ScannerDiagnosticsDrawer
        open={isDiagnosticsOpen}
        onOpenChange={setIsDiagnosticsOpen}
      />
        </div>
      </DialogContent>
    </Dialog>
  );
}
