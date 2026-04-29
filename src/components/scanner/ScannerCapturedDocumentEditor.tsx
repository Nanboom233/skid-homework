import {type PointerEvent as ReactPointerEvent, useCallback, useEffect, useMemo, useRef, useState} from "react";
import {useTranslation} from "react-i18next";

import type {Point} from "@/lib/scanner";
import type {ScannerCapturedDocument} from "@/store/scanner-store";
import type {ScannerPostProcessBackend} from "@/store/settings-store";
import {useSettingsStore} from "@/store/settings-store";
import {Button} from "@/components/ui/button";
import {Switch} from "@/components/ui/switch";
import {Label} from "@/components/ui/label";
import {Badge} from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {useBlobDataUrl} from "@/hooks/use-blob-data-url";
import {mapPointFromSourceToRotatedFrame, mapPointFromRotatedFrameToSource} from "@/lib/scanner/preview-orientation";
import {refineDocumentCorners} from "@/lib/tauri/scanner";

export type EditorColorMode = "auto" | "color" | "grayscale" | "binary";

export interface PostProcessOptions {
  imageEnhancement: boolean;
  colorMode: EditorColorMode;
  postprocessBackend: ScannerPostProcessBackend;
  spineFlattening: boolean;
  perspectiveTransform?: boolean;
  gridPostprocess?: "none" | "x-stretch-equalize";
}

interface ScannerCapturedDocumentEditorProps {
  open: boolean;
  document: ScannerCapturedDocument | null;
  isApplying: boolean;
  onOpenChange: (open: boolean) => void;
  onApply: (documentId: string, points: Point[], options: PostProcessOptions) => void;
  onPreviewRequest?: (
    document: ScannerCapturedDocument,
    points: Point[],
    options: PostProcessOptions,
  ) => Promise<{ blob: Blob; processingMs: number }>;
}

interface ScannerCapturedDocumentEditorBodyProps {
  document: ScannerCapturedDocument;
  isApplying: boolean;
  onOpenChange: (open: boolean) => void;
  onApply: (documentId: string, points: Point[], options: PostProcessOptions) => void;
  onPreviewRequest?: (
    document: ScannerCapturedDocument,
    points: Point[],
    options: PostProcessOptions,
  ) => Promise<{ blob: Blob; processingMs: number }>;
}

const DEFAULT_INSET_RATIO = 0.08;
const CORNER_LABELS = ["TL", "TR", "BR", "BL"] as const;
const EDITOR_CANVAS_MAX_WIDTH_PX = 760;
const EDITOR_CANVAS_MAX_HEIGHT_RATIO = 0.48;

type OrthogonalRotation = ScannerCapturedDocument["outputRotation"];

const clonePoints = (points: Point[] | null): Point[] | null => {
  return points?.map((point) => ({...point})) ?? null;
};

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(max, Math.max(min, value));
};

const getRotatedDimensions = (
  width: number,
  height: number,
  rotation: OrthogonalRotation,
): { width: number; height: number } => {
  if (rotation === 90 || rotation === 270) {
    return { width: height, height: width };
  }
  return { width, height };
};

const buildDefaultPoints = (width: number, height: number): Point[] => {
  const insetX = Math.max(12, width * DEFAULT_INSET_RATIO);
  const insetY = Math.max(12, height * DEFAULT_INSET_RATIO);
  return [
    {x: insetX, y: insetY},
    {x: width - insetX, y: insetY},
    {x: width - insetX, y: height - insetY},
    {x: insetX, y: height - insetY},
  ];
};

const getViewportSize = (): { width: number; height: number } => {
  if (typeof window === "undefined") {
    return { width: 1000, height: 900 };
  }
  return { width: window.innerWidth, height: window.innerHeight };
};

const getInitialPoints = (document: ScannerCapturedDocument): Point[] => {
  const existing = clonePoints(document.points);
  const raw = (existing && existing.length === 4)
    ? existing
    : buildDefaultPoints(document.sourceWidth, document.sourceHeight);

  // Detection returns points in source (un-rotated) coords.
  // Convert them to the rotated coordinate space used by the editor.
  if (document.outputRotation === 0) return raw;
  return raw.map((p) => mapPointFromSourceToRotatedFrame(
    p,
    document.sourceWidth,
    document.sourceHeight,
    document.outputRotation,
  ));
};

/**
 * Convert editor draft points (rotated display space) back to source-image
 * coordinate space.  All outgoing calls (Apply, Refine, Preview) must funnel
 * through this so the pipeline contract ("points are in source space") holds.
 */
const draftPointsToSourceSpace = (
  points: Point[],
  sourceWidth: number,
  sourceHeight: number,
  rotation: OrthogonalRotation,
): Point[] => {
  if (rotation === 0) return points.map((p) => ({...p}));
  return points.map((p) => mapPointFromRotatedFrameToSource(
    p, sourceWidth, sourceHeight, rotation,
  ));
};

const buildDocumentEditorKey = (document: ScannerCapturedDocument): string => {
  const pointsKey = document.points?.map((point) => `${point.x}:${point.y}`).join("|") ?? "none";
  return `${document.id}:${document.sourceWidth}x${document.sourceHeight}:${document.outputRotation}:${pointsKey}`;
};

/** Serialise options + points into a stable fingerprint for dirty-checking. */
const buildOptionsFingerprint = (
  points: Point[],
  options: PostProcessOptions,
): string => {
  const pointsStr = points.map((p) => `${Math.round(p.x * 10)},${Math.round(p.y * 10)}`).join("|");
  return `${pointsStr}::${options.imageEnhancement}:${options.colorMode}:${options.postprocessBackend}:${options.spineFlattening}:${options.perspectiveTransform}:${options.gridPostprocess}`;
};

export function ScannerCapturedDocumentEditor({
  open,
  document,
  isApplying,
  onOpenChange,
  onApply,
  onPreviewRequest,
}: ScannerCapturedDocumentEditorProps) {
  if (!open || !document) return null;

  return (
    <div className="absolute inset-0 z-20 flex flex-col overflow-hidden bg-background">
      <ScannerCapturedDocumentEditorBody
        key={buildDocumentEditorKey(document)}
        document={document}
        isApplying={isApplying}
        onOpenChange={onOpenChange}
        onApply={onApply}
        onPreviewRequest={onPreviewRequest}
      />
    </div>
  );
}

function ScannerCapturedDocumentEditorBody({
  document,
  isApplying,
  onOpenChange,
  onApply,
  onPreviewRequest,
}: ScannerCapturedDocumentEditorBodyProps) {
  const {t} = useTranslation("commons");
  const svgRef = useRef<SVGSVGElement | null>(null);
  const activeCornerIndexRef = useRef<number | null>(null);
  const [draftPoints, setDraftPoints] = useState<Point[]>(() => getInitialPoints(document));
  const [viewportSize, setViewportSize] = useState(() => getViewportSize());
  const sourceUrl = useBlobDataUrl(document.sourceFile);
  const displayDimensions = getRotatedDimensions(
    document.sourceWidth,
    document.sourceHeight,
    document.outputRotation,
  );

  // Post-processing options — initialized from document overrides, falling back to global settings
  const globalEnhancement = useSettingsStore((state) => state.imageEnhancement);
  const globalBackend = useSettingsStore((state) => state.scannerPostProcessBackend);
  const [imageEnhancement, setImageEnhancement] = useState(document.options?.imageEnhancement ?? globalEnhancement);
  const [colorMode, setColorMode] = useState<EditorColorMode>(document.options?.colorMode ?? "auto");
  const [postprocessBackend, setPostprocessBackend] = useState<ScannerPostProcessBackend>(document.options?.postprocessBackend ?? globalBackend);
  const [spineFlattening, setSpineFlattening] = useState(document.options?.spineFlattening ?? true);
  const [perspectiveTransform, setPerspectiveTransform] = useState(document.options?.perspectiveTransform ?? true);
  const [gridPostprocess, setGridPostprocess] = useState<"none" | "x-stretch-equalize">(document.options?.gridPostprocess ?? "none");

  // Corner refinement state
  const [isRefining, setIsRefining] = useState(false);

  // Preview state
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewProcessingMs, setPreviewProcessingMs] = useState<number | null>(null);
  const [previewFullscreen, setPreviewFullscreen] = useState(false);
  /** Fingerprint of the options that produced the current preview. */
  const lastPreviewFingerprintRef = useRef<string | null>(null);

  // Auto-load the existing processed result as initial preview
  const existingFileUrl = useBlobDataUrl(
    document.status === "ready" ? document.file : null,
  );
  const hasCustomPreview = previewUrl !== null;

  // Recommendation based on document analysis
  const recommendEnhancement = document.documentDetected;

  // Current options bundle
  const currentOptions = useMemo<PostProcessOptions>(() => ({
    imageEnhancement,
    colorMode,
    postprocessBackend,
    spineFlattening,
    perspectiveTransform,
    gridPostprocess,
  }), [imageEnhancement, colorMode, postprocessBackend, spineFlattening, perspectiveTransform, gridPostprocess]);

  // Current fingerprint — used to detect whether a re-process is needed
  const currentFingerprint = useMemo(
    () => buildOptionsFingerprint(draftPoints, currentOptions),
    [draftPoints, currentOptions],
  );

  const isDirty = lastPreviewFingerprintRef.current !== null
    && lastPreviewFingerprintRef.current !== currentFingerprint;

  useEffect(() => {
    const updateViewportSize = (): void => {
      setViewportSize(getViewportSize());
    };
    updateViewportSize();
    window.addEventListener("resize", updateViewportSize);
    return () => {
      window.removeEventListener("resize", updateViewportSize);
    };
  }, []);

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent): void => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      if (previewFullscreen) {
        setPreviewFullscreen(false);
      } else {
        onOpenChange(false);
      }
    };
    window.document.addEventListener("keydown", handleEscape);
    return () => window.document.removeEventListener("keydown", handleEscape);
  }, [onOpenChange, previewFullscreen]);

  // Clean up custom preview URL on unmount
  useEffect(() => {
    return () => {
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
    };
  }, [previewUrl]);

  // Mark existing file as the initial "baseline" so isDirty starts false
  useEffect(() => {
    if (existingFileUrl && lastPreviewFingerprintRef.current === null) {
      // Treat the initial state as already-previewed
      lastPreviewFingerprintRef.current = currentFingerprint;
    }
  }, [existingFileUrl, currentFingerprint]);

  const updatePointFromEvent = (event: ReactPointerEvent<SVGSVGElement>): void => {
    const svg = svgRef.current;
    const activeCornerIndex = activeCornerIndexRef.current;
    if (!svg || activeCornerIndex === null) return;

    const rect = svg.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;

    const x = clamp(
      ((event.clientX - rect.left) / rect.width) * displayDimensions.width,
      0, displayDimensions.width,
    );
    const y = clamp(
      ((event.clientY - rect.top) / rect.height) * displayDimensions.height,
      0, displayDimensions.height,
    );

    setDraftPoints((current) => current.map((point, index) => {
      if (index !== activeCornerIndex) return point;
      return {
        x: clamp(x, 0, displayDimensions.width),
        y: clamp(y, 0, displayDimensions.height),
      };
    }));
  };

  const handlePointerDown = (
    index: number,
    event: ReactPointerEvent<SVGCircleElement>,
  ): void => {
    activeCornerIndexRef.current = index;
    svgRef.current?.setPointerCapture(event.pointerId);
  };

  const handlePointerMove = (event: ReactPointerEvent<SVGSVGElement>): void => {
    if (activeCornerIndexRef.current === null) return;
    updatePointFromEvent(event);
  };

  const handlePointerUp = (event: ReactPointerEvent<SVGSVGElement>): void => {
    if (svgRef.current?.hasPointerCapture(event.pointerId)) {
      svgRef.current.releasePointerCapture(event.pointerId);
    }
    activeCornerIndexRef.current = null;
  };

  const handleReset = (): void => {
    setDraftPoints(getInitialPoints(document));
  };

  const handleApply = (): void => {
    if (draftPoints.length !== 4) return;
    const sourceSpacePoints = draftPointsToSourceSpace(
      draftPoints, document.sourceWidth, document.sourceHeight, document.outputRotation,
    );
    onApply(document.id, sourceSpacePoints, currentOptions);
  };

  const handlePreview = useCallback(async () => {
    if (!onPreviewRequest || draftPoints.length !== 4 || previewLoading) return;

    // Skip if nothing changed since last preview
    if (lastPreviewFingerprintRef.current === currentFingerprint && hasCustomPreview) return;

    setPreviewLoading(true);
    setPreviewProcessingMs(null);

    try {
      const sourceSpacePoints = draftPointsToSourceSpace(
        draftPoints, document.sourceWidth, document.sourceHeight, document.outputRotation,
      );
      const result = await onPreviewRequest(document, sourceSpacePoints, currentOptions);

      // Revoke previous custom URL
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }

      const url = URL.createObjectURL(result.blob);
      setPreviewUrl(url);
      setPreviewProcessingMs(Math.round(result.processingMs));
      lastPreviewFingerprintRef.current = currentFingerprint;
    } catch (error) {
      console.error("[Scanner][Editor] Preview request failed:", error);
    } finally {
      setPreviewLoading(false);
    }
  }, [onPreviewRequest, draftPoints, currentOptions, currentFingerprint, previewLoading, hasCustomPreview, document, previewUrl]);

  // Displayed preview URL: custom preview > existing file
  const displayedPreviewUrl = hasCustomPreview ? previewUrl : existingFileUrl;

  const polygonPoints = draftPoints.map((point) => `${point.x},${point.y}`).join(" ");
  const referenceSize = Math.max(document.sourceWidth, document.sourceHeight);
  const strokeWidth = Math.max(3, referenceSize * 0.005);
  const handleRadius = Math.max(9, referenceSize * 0.014);
  const availableWidth = Math.max(
    240,
    Math.min(
      EDITOR_CANVAS_MAX_WIDTH_PX,
      viewportSize.width - (viewportSize.width < 640 ? 40 : 160),
    ),
  );
  const availableHeight = Math.max(
    240,
    Math.min(viewportSize.height * EDITOR_CANVAS_MAX_HEIGHT_RATIO, 440),
  );
  const previewScale = Math.min(
    1,
    availableWidth / Math.max(1, displayDimensions.width),
    availableHeight / Math.max(1, displayDimensions.height),
  );
  const previewWidth = Math.max(1, Math.round(displayDimensions.width * previewScale));
  const previewHeight = Math.max(1, Math.round(displayDimensions.height * previewScale));
  const sourcePreviewWidth = Math.max(1, Math.round(document.sourceWidth * previewScale));
  const sourcePreviewHeight = Math.max(1, Math.round(document.sourceHeight * previewScale));

  return (
    <div className="flex h-full min-h-0 flex-col" role="dialog" aria-modal="true" aria-labelledby="scanner-editor-title" aria-describedby="scanner-editor-description">
      <div className="flex flex-col space-y-1.5 border-b p-4 sm:p-6 shrink-0">
        <h2 id="scanner-editor-title" className="text-lg font-semibold leading-none tracking-tight">{t("document-scanner.editor.title")}</h2>
        <p id="scanner-editor-description" className="text-sm text-muted-foreground">{t("document-scanner.editor.description")}</p>
      </div>

      <div className="flex-1 overflow-y-auto p-4 sm:p-6">
      <div className="space-y-4">
        {/* Corner editor canvas */}
        <div className="rounded-xl border bg-muted/20 p-3">
          <div className="flex justify-center overflow-auto">
            <div
              className="relative shrink-0 overflow-hidden rounded-lg border border-white/10 bg-black shadow-inner"
              style={{ width: `${previewWidth}px`, height: `${previewHeight}px` }}
            >
              {/* Image layer — CSS-rotated to display the un-rotated source correctly */}
              <div
                className="absolute left-1/2 top-1/2"
                style={{
                  width: `${sourcePreviewWidth}px`,
                  height: `${sourcePreviewHeight}px`,
                  transform: `translate(-50%, -50%) rotate(${document.outputRotation}deg)`,
                  transformOrigin: "center center",
                }}
              >
                {sourceUrl ? (
                  <>
                    {/* eslint-disable-next-line @next/next/no-img-element */}
                    <img
                      src={sourceUrl}
                      alt={t("document-scanner.editor.image-alt")}
                      className="absolute inset-0 h-full w-full object-fill"
                      draggable={false}
                    />
                  </>
                ) : (
                  <div className="absolute inset-0 bg-muted/20" />
                )}
              </div>
              {/* SVG overlay — uses rotated coordinate space directly */}
              <svg
                ref={svgRef}
                className="absolute inset-0 h-full w-full touch-none"
                viewBox={`0 0 ${displayDimensions.width} ${displayDimensions.height}`}
                onPointerMove={handlePointerMove}
                onPointerUp={handlePointerUp}
                onPointerCancel={handlePointerUp}
              >
                <polygon
                  points={polygonPoints}
                  fill="rgba(59, 130, 246, 0.18)"
                  stroke="rgba(96, 165, 250, 0.92)"
                  strokeWidth={strokeWidth}
                  strokeLinejoin="round"
                />
                {draftPoints.map((point, index) => (
                  <g key={`editor-corner-${index}`}>
                    <circle
                      cx={point.x}
                      cy={point.y}
                      r={handleRadius}
                      fill="white"
                      stroke="rgba(37, 99, 235, 0.95)"
                      strokeWidth={Math.max(2, referenceSize * 0.003)}
                      onPointerDown={(event) => handlePointerDown(index, event)}
                    />
                    <text
                      x={point.x}
                      y={point.y - handleRadius - Math.max(10, referenceSize * 0.01)}
                      textAnchor="middle"
                      fontSize={Math.max(14, referenceSize * 0.018)}
                      fontWeight="700"
                      fill="white"
                      stroke="rgba(0,0,0,0.45)"
                      strokeWidth={Math.max(1.5, referenceSize * 0.0015)}
                      paintOrder="stroke"
                    >
                      {CORNER_LABELS[index]}
                    </text>
                  </g>
                ))}
              </svg>
            </div>
          </div>
        </div>

        <p className="text-xs text-muted-foreground">
          {t("document-scanner.editor.hint")}
        </p>

        {/* Post-processing options */}
        <div className="rounded-xl border bg-muted/10 p-4 space-y-3">
          <h4 className="text-sm font-semibold text-foreground">
            {t("document-scanner.editor.postprocess.title")}
          </h4>

          {/* Image Enhancement */}
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Label htmlFor="editor-enhancement" className="text-sm cursor-pointer">
                {t("document-scanner.editor.postprocess.image-enhancement")}
              </Label>
              <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                {recommendEnhancement
                  ? t("document-scanner.editor.postprocess.recommended-on")
                  : t("document-scanner.editor.postprocess.recommended-off")}
              </Badge>
            </div>
            <Switch
              id="editor-enhancement"
              checked={imageEnhancement}
              onCheckedChange={setImageEnhancement}
              disabled={isApplying}
            />
          </div>

          {/* Spine Flattening */}
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="editor-flattening" className="text-sm cursor-pointer">
              {t("document-scanner.editor.postprocess.spine-flattening")}
            </Label>
            <Switch
              id="editor-flattening"
              checked={spineFlattening}
              onCheckedChange={setSpineFlattening}
              disabled={isApplying}
            />
          </div>

          {/* Refine Corners Button */}
          {draftPoints.length === 4 ? (
            <Button
              variant="outline"
              size="sm"
              className="w-full text-xs"
              disabled={isApplying || isRefining}
              onClick={async () => {
                setIsRefining(true);
                try {
                  // Convert rotated display space → source space for the Rust backend
                  const sourceSpaceInput = draftPointsToSourceSpace(
                    draftPoints, document.sourceWidth, document.sourceHeight, document.outputRotation,
                  );
                  const refined = await refineDocumentCorners(document.sourceFile, sourceSpaceInput);
                  if (refined.length === 4) {
                    // Convert refined result (source space) back to rotated display space
                    setDraftPoints(
                      document.outputRotation === 0
                        ? refined
                        : refined.map((p) => mapPointFromSourceToRotatedFrame(
                            p, document.sourceWidth, document.sourceHeight, document.outputRotation,
                          )),
                    );
                  }
                } catch (error) {
                  console.error("Corner refinement failed:", error);
                } finally {
                  setIsRefining(false);
                }
              }}
            >
              {isRefining ? "Refining..." : "Refine Corners"}
            </Button>
          ) : null}

          {/* Perspective Transform */}
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="editor-perspective" className="text-sm cursor-pointer">
              Perspective Transform
            </Label>
            <Switch
              id="editor-perspective"
              checked={perspectiveTransform}
              onCheckedChange={setPerspectiveTransform}
              disabled={isApplying}
            />
          </div>

            <div className="flex items-center justify-between gap-3">
              <Label htmlFor="editor-grid-postprocess" className="text-sm shrink-0">
                Grid Post-Process
              </Label>
              <Select
                value={gridPostprocess}
                onValueChange={(value) => setGridPostprocess(value as "none" | "x-stretch-equalize")}
                disabled={isApplying}
              >
                <SelectTrigger id="editor-grid-postprocess" className="w-[160px] h-8 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None (raw)</SelectItem>
                  <SelectItem value="x-stretch-equalize">X-Stretch Equalize</SelectItem>
                </SelectContent>
              </Select>
            </div>

          {/* Color Mode */}
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="editor-color-mode" className="text-sm shrink-0">
              {t("document-scanner.editor.postprocess.color-mode")}
            </Label>
            <Select
              value={colorMode}
              onValueChange={(value) => setColorMode(value as EditorColorMode)}
              disabled={isApplying}
            >
              <SelectTrigger id="editor-color-mode" className="w-[140px] h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">
                  {t("document-scanner.editor.postprocess.color-mode-options.auto")}
                </SelectItem>
                <SelectItem value="color">
                  {t("document-scanner.editor.postprocess.color-mode-options.color")}
                </SelectItem>
                <SelectItem value="grayscale">
                  {t("document-scanner.editor.postprocess.color-mode-options.grayscale")}
                </SelectItem>
                <SelectItem value="binary">
                  {t("document-scanner.editor.postprocess.color-mode-options.binary")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          {/* Post-process Backend */}
          <div className="flex items-center justify-between gap-3">
            <Label htmlFor="editor-pp-backend" className="text-sm shrink-0">
              {t("document-scanner.editor.postprocess.postprocess-backend")}
            </Label>
            <Select
              value={postprocessBackend}
              onValueChange={(value) => setPostprocessBackend(value as ScannerPostProcessBackend)}
              disabled={isApplying}
            >
              <SelectTrigger id="editor-pp-backend" className="w-[140px] h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="heuristic">
                  {t("document-scanner.editor.postprocess.postprocess-backend-options.heuristic")}
                </SelectItem>
                <SelectItem value="native-ml-v1">
                  {t("document-scanner.editor.postprocess.postprocess-backend-options.native-ml-v1")}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        {/* Preview section */}
        <div className="rounded-xl border bg-muted/10 p-4 space-y-3">
          <div className="flex items-center gap-2 flex-wrap">
            {onPreviewRequest ? (
              <Button
                variant="outline"
                size="sm"
                onClick={handlePreview}
                disabled={draftPoints.length !== 4 || previewLoading || isApplying}
                className="text-xs"
              >
                {previewLoading
                  ? t("document-scanner.editor.postprocess.preview.loading")
                  : t("document-scanner.editor.postprocess.preview.button")}
              </Button>
            ) : null}
            {previewProcessingMs !== null ? (
              <span className="text-[11px] text-muted-foreground">
                {t("document-scanner.editor.postprocess.preview.processing-time", {
                  ms: previewProcessingMs,
                })}
              </span>
            ) : null}
            {isDirty && !previewLoading ? (
              <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
                {t("document-scanner.editor.postprocess.preview.stale")}
              </Badge>
            ) : null}
          </div>

          {displayedPreviewUrl ? (
            <button
              type="button"
              className="relative w-full max-h-[200px] rounded-lg overflow-hidden border border-white/10 bg-black/40 cursor-zoom-in group"
              onClick={() => setPreviewFullscreen(true)}
            >
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img
                src={displayedPreviewUrl}
                alt={t("document-scanner.editor.postprocess.preview.fullscreen-alt")}
                className="w-full h-full object-contain max-h-[200px]"
              />
              <span className="absolute bottom-1 right-2 text-[10px] text-white/60 opacity-0 group-hover:opacity-100 transition-opacity">
                {t("document-scanner.editor.postprocess.preview.click-to-enlarge")}
              </span>
            </button>
          ) : null}
        </div>
      </div>
      </div>

      <div className="flex flex-col-reverse gap-2 border-t p-4 sm:flex-row sm:justify-between sm:p-6 shrink-0 bg-background">
        <Button variant="outline" onClick={handleReset} disabled={isApplying}>
          {t("document-scanner.editor.actions.reset")}
        </Button>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={isApplying}>
            {t("document-scanner.editor.actions.cancel")}
          </Button>
          <Button onClick={handleApply} disabled={draftPoints.length !== 4 || isApplying}>
            {isApplying
              ? t("document-scanner.editor.actions.applying")
              : t("document-scanner.editor.actions.apply")}
          </Button>
        </div>
      </div>

      {previewFullscreen && displayedPreviewUrl && (
        <div
          ref={(el) => el?.focus()}
          className="fixed inset-0 z-[100] flex items-center justify-center bg-black/95 backdrop-blur-sm"
          onClick={() => setPreviewFullscreen(false)}
          tabIndex={-1}
          role="dialog"
          aria-modal="true"
          aria-label={t("document-scanner.editor.postprocess.preview.fullscreen-alt")}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={displayedPreviewUrl}
            alt={t("document-scanner.editor.postprocess.preview.fullscreen-alt")}
            className="max-w-[95vw] max-h-[90vh] object-contain cursor-zoom-out"
            onClick={() => setPreviewFullscreen(false)}
          />
        </div>
      )}
    </div>
  );
}
