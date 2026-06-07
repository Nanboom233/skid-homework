"use client";

import {useState} from "react";
import {Activity, ChevronDown, ChevronUp, TimerReset, TriangleAlert, Wifi} from "lucide-react";
import {useTranslation} from "react-i18next";

import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from "@/components/ui/card";
import {Collapsible, CollapsibleContent, CollapsibleTrigger} from "@/components/ui/collapsible";
import {Separator} from "@/components/ui/separator";
import {useScannerStore} from "../../store/scanner-store";

const CORNER_LABELS = ["TL", "TR", "BR", "BL"] as const;

const formatResolution = (width: number | null, height: number | null): string => {
  if (!width || !height) return "—";
  return `${width} × ${height}`;
};

const formatNumber = (value: number | null, fractionDigits: number = 1, suffix: string = ""): string => {
  if (value === null || Number.isNaN(value)) return "—";
  return `${value.toFixed(fractionDigits)}${suffix}`;
};

const formatPayload = (payloadBytes: number | null): string => {
  if (payloadBytes === null || Number.isNaN(payloadBytes)) return "—";
  return `${(payloadBytes / 1024).toFixed(1)} KB`;
};

const FPS_ACCEPTANCE_THRESHOLD = 30;

const getFpsBenchmarkState = (fps: number | null) => {
  if (fps === null || Number.isNaN(fps)) return { label: "pending", variant: "outline" as const };
  if (fps >= FPS_ACCEPTANCE_THRESHOLD) return { label: "pass", variant: "default" as const };
  return { label: "fail", variant: "destructive" as const };
};

const formatTimestamp = (timestamp: number | null): string => {
  if (!timestamp) return "—";
  return new Date(timestamp).toLocaleTimeString();
};

const getReconnectVariant = (state: string) => {
  switch (state) {
    case "connected": return "default";
    case "connecting":
    case "reconnecting": return "secondary";
    case "error": return "destructive";
    default: return "outline";
  }
};

const getPostProcessVariant = (state: string) => {
  switch (state) {
    case "success": return "default";
    case "processing": return "secondary";
    case "error": return "destructive";
    default: return "outline";
  }
};

const getFpsBenchmarkLabelKey = (label: string) => {
  switch (label) {
    case "pass": return "debug.badges.state.pass";
    case "fail": return "debug.badges.state.fail";
    default: return "debug.badges.state.pending";
  }
};

const getCvReadyBadgeKey = (backend: string) => {
  switch (backend) {
    case "native-ort": return "debug.cv.badges.ready-native-ort";
    default: return "debug.cv.badges.ready-opencv";
  }
};

const getPostProcessBadgeKey = (state: string) => {
  switch (state) {
    case "processing": return "debug.capture.badges.processing";
    case "success": return "debug.capture.badges.success";
    case "error": return "debug.capture.badges.error";
    default: return "debug.capture.badges.idle";
  }
};


interface MetricItemProps {
  label: string;
  value: string;
  hint?: string;
}

const MetricItem = ({ label, value, hint }: MetricItemProps) => (
  <div className="min-w-0 rounded-lg border bg-background/60 p-3">
    <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{label}</p>
    <p className="mt-1 break-words text-sm font-semibold text-foreground">{value}</p>
    {hint ? <p className="mt-1 break-words text-[11px] text-muted-foreground">{hint}</p> : null}
  </div>
);

const useScannerDebugModel = () => {
  const { t } = useTranslation("commons", { keyPrefix: "document-scanner" });
  const errorMessage = useScannerStore((state) => state.errorMessage);
  const previewDebug = useScannerStore((state) => state.previewDebug);
  const cvDebug = useScannerStore((state) => state.cvDebug);
  const connectionDebug = useScannerStore((state) => state.connectionDebug);
  const captureDebug = useScannerStore((state) => state.captureDebug);

  const currentFpsBenchmark = getFpsBenchmarkState(previewDebug.previewFps);

  const translateReconnectState = (value: string): string => {
    switch (value) {
      case "connected":
      case "connecting":
      case "reconnecting":
      case "stopped":
      case "error":
      case "idle":
      case "starting":
      case "stopping": return t(`debug.states.reconnect.${value}`);
      default: return value;
    }
  };

  const translateBackendState = (value: string): string => {
    switch (value) {
      case "opencv":
      case "native-ort": return t(`debug.cv.backends.${value}`);
      default: return value;
    }
  };

  const translateModelKind = (value: string | null): string => {
    switch (value) {
      case "public-baseline":
      case "planned-primary": return t(`debug.cv.model-kind.${value}`);
      default: return value ?? "—";
    }
  };

  return {
    t,
    status,
    errorMessage,
    previewDebug,
    cvDebug,
    connectionDebug,
    captureDebug,
    currentFpsBenchmark,
    translateReconnectState,
    translateBackendState,
    translateModelKind,
  };
};

export function ScannerDetectionDebugCard() {
  const {
    t,
    errorMessage,
    previewDebug,
    cvDebug,
    connectionDebug,
    currentFpsBenchmark,
    translateReconnectState,
    translateBackendState,
    translateModelKind,
  } = useScannerDebugModel();

  const [isOpen, setIsOpen] = useState(false);

  return (
    <Card className="min-w-0 w-full gap-0">
      <Collapsible open={isOpen} onOpenChange={setIsOpen}>
        <CardHeader className="pb-4 pt-4 sm:pt-6 group">
          <div className="flex items-start justify-between">
            <div className="flex flex-col gap-1.5">
              <CardTitle className="flex items-center gap-2 text-base">
                <Activity className="h-4 w-4" />
                {t("debug.detection.title")}
              </CardTitle>
              <CardDescription>
                {t("debug.detection.description")}
              </CardDescription>
            </div>
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm" className="w-9 p-0">
                {isOpen ? (
                  <ChevronUp className="h-4 w-4" />
                ) : (
                  <ChevronDown className="h-4 w-4" />
                )}
                <span className="sr-only">Toggle</span>
              </Button>
            </CollapsibleTrigger>
          </div>

          <div className="flex flex-wrap items-center gap-2 pt-2">
            <Badge variant={cvDebug.cvReady ? "default" : "destructive"}>
              {cvDebug.cvReady
                ? t(getCvReadyBadgeKey(cvDebug.activeBackend))
                : t("debug.cv.badges.unavailable")}
            </Badge>
            <Badge variant={getReconnectVariant(connectionDebug.reconnectState)}>
              <Wifi className="h-3 w-3" />
              {translateReconnectState(connectionDebug.reconnectState)}
            </Badge>
            <Badge variant={cvDebug.documentDetected ? "default" : "outline"}>
              {cvDebug.documentDetected ? t("debug.cv.badges.document-detected") : t("debug.cv.badges.no-document")}
            </Badge>
            <Badge variant={cvDebug.isStable ? "default" : "outline"}>
              {cvDebug.isStable ? t("debug.cv.badges.stable") : t("debug.cv.badges.unstable")}
            </Badge>
            <Badge variant={currentFpsBenchmark.variant}>
              {t("debug.badges.current", {
                state: t(getFpsBenchmarkLabelKey(currentFpsBenchmark.label)),
                fps: Math.round(previewDebug.previewFps ?? 0),
              })}
            </Badge>
            <p className="text-xs text-muted-foreground ml-1">
              {formatResolution(previewDebug.previewWidth, previewDebug.previewHeight)}
            </p>
          </div>
        </CardHeader>

        <CollapsibleContent>
          <CardContent className="space-y-4 pt-0">
            <Separator />
            <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
              <MetricItem
                label={t("debug.metrics.current-fps.label")}
                value={formatNumber(previewDebug.previewFps, 1)}
                hint={t("debug.metrics.current-fps.hint")}
              />
              <MetricItem
                label={t("debug.metrics.window-fps.label")}
                value={formatNumber(previewDebug.recentWindowFps, 1)}
                hint={t("debug.metrics.window-fps.hint")}
              />
              <MetricItem
                label={t("debug.metrics.effective-fps.label")}
                value={formatNumber(previewDebug.effectiveFps, 1)}
                hint={t("debug.metrics.effective-fps.hint")}
              />
              <MetricItem
                label={t("debug.metrics.frame-index")}
                value={String(previewDebug.frameIndex)}
              />
              <MetricItem
                label={t("debug.metrics.payload-size")}
                value={formatPayload(previewDebug.payloadBytes)}
              />
              <MetricItem
                label={t("debug.metrics.poll-count")}
                value={previewDebug.pollCount === null ? "—" : String(previewDebug.pollCount)}
              />
              <MetricItem
                label={t("debug.metrics.poll-wait")}
                value={formatNumber(previewDebug.pollWaitMs, 1, " ms")}
              />
              <MetricItem
                label={t("debug.metrics.js-decode")}
                value={formatNumber(previewDebug.jsDecodeMs, 1, " ms")}
              />
              <MetricItem
                label={t("debug.metrics.canvas-draw")}
                value={formatNumber(previewDebug.canvasDrawMs, 1, " ms")}
              />
            </div>

            <Separator />
            <div className="grid gap-3 sm:grid-cols-2">
              <MetricItem
                label={t("debug.cv.metrics.active-backend")}
                value={translateBackendState(cvDebug.activeBackend)}
                hint={cvDebug.backendMessage ?? undefined}
              />
              <MetricItem
                label={t("debug.cv.metrics.requested-backend")}
                value={translateBackendState(cvDebug.requestedBackend)}
                hint={cvDebug.strictMode ? t("debug.cv.metrics.strict-mode-on") : t("debug.cv.metrics.strict-mode-off")}
              />
              <MetricItem
                label={t("debug.cv.metrics.stage1-model")}
                value={cvDebug.selectedModelId ?? "—"}
                hint={translateModelKind(cvDebug.selectedModelKind)}
              />
              <MetricItem
                label={t("debug.cv.metrics.provider")}
                value={cvDebug.preferredProvider ?? "—"}
                hint={cvDebug.preferredProvider ? (cvDebug.preferredProviderReady ? t("debug.cv.metrics.provider-ready") : t("debug.cv.metrics.provider-not-ready")) : undefined}
              />
              <MetricItem
                label={t("debug.metrics.reconnect-status")}
                value={translateReconnectState(connectionDebug.reconnectState)}
                hint={connectionDebug.reconnectAttempt !== null ? t("debug.hints.reconnect-attempt", { attempt: connectionDebug.reconnectAttempt, max: connectionDebug.reconnectMaxAttempts ?? "—" }) : connectionDebug.reconnectMessage ?? t("debug.hints.no-reconnect-attempt")}
              />
              <MetricItem
                label={t("debug.metrics.recent-error")}
                value={connectionDebug.lastErrorReason ?? errorMessage ?? "—"}
                hint={connectionDebug.lastDisconnectAt ? t("debug.hints.last-disconnect", { time: formatTimestamp(connectionDebug.lastDisconnectAt) }) : t("debug.hints.no-disconnect")}
              />
            </div>

            <Separator />
            {cvDebug.cornerPoints.length > 0 ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {cvDebug.cornerPoints.map((point, index) => (
                  <MetricItem
                    key={`${CORNER_LABELS[index] ?? index}-${point.x}-${point.y}`}
                    label={t("debug.cv.metrics.corner-label", { corner: CORNER_LABELS[index] ?? index + 1 })}
                    value={`${Math.round(point.x)}, ${Math.round(point.y)}`}
                  />
                ))}
              </div>
            ) : (
              <div className="rounded-xl border border-dashed bg-muted/30 p-4 text-sm text-muted-foreground">
                <div className="flex items-start gap-2">
                  <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                  <p>{t("debug.cv.empty")}</p>
                </div>
              </div>
            )}
          </CardContent>
        </CollapsibleContent>
      </Collapsible>
    </Card>
  );
}

export function ScannerPostProcessDetailsCard() {
  const { t, captureDebug } = useScannerDebugModel();

  return (
    <Card className="min-w-0 w-full gap-0">
      <CardHeader className="pb-4">
        <CardTitle className="flex items-center gap-2 text-base">
          <TimerReset className="h-4 w-4" />
          {t("debug.capture.title")}
        </CardTitle>
        <CardDescription>
          {t("debug.capture.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={getPostProcessVariant(captureDebug.postProcessStatus)}>
            {t(getPostProcessBadgeKey(captureDebug.postProcessStatus))}
          </Badge>
          <Badge variant={captureDebug.postProcessUsedRedetect ? "secondary" : "outline"}>
            {captureDebug.postProcessUsedRedetect ? t("debug.capture.badges.redetect-on") : t("debug.capture.badges.redetect-off")}
          </Badge>
          <Badge variant={captureDebug.postProcessUsedPerspective ? "secondary" : "outline"}>
            {captureDebug.postProcessUsedPerspective ? t("debug.capture.badges.crop-on") : t("debug.capture.badges.crop-off")}
          </Badge>
          <Badge variant={captureDebug.postProcessUsedEnhancement ? "secondary" : "outline"}>
            {captureDebug.postProcessUsedEnhancement ? t("debug.capture.badges.enhance-on") : t("debug.capture.badges.enhance-off")}
          </Badge>
          <Badge variant={captureDebug.postProcessUsedResidualWarp ? "secondary" : "outline"}>
            {captureDebug.postProcessUsedResidualWarp ? t("debug.capture.badges.warp-on") : t("debug.capture.badges.warp-off")}
          </Badge>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3">
          <MetricItem
            label={t("debug.capture.metrics.total")}
            value={formatNumber(captureDebug.postProcessTotalMs, 1, " ms")}
            hint={t("debug.capture.metrics.total-hint")}
          />
          <MetricItem
            label={t("debug.capture.metrics.decode")}
            value={formatNumber(captureDebug.postProcessDecodeMs, 1, " ms")}
          />
          <MetricItem
            label={t("debug.capture.metrics.redetect")}
            value={formatNumber(captureDebug.postProcessRedetectMs, 1, " ms")}
          />
          <MetricItem
            label={t("debug.capture.metrics.crop")}
            value={formatNumber(captureDebug.postProcessPerspectiveMs, 1, " ms")}
          />

          <MetricItem
            label={t("debug.capture.metrics.model-ms")}
            value={formatNumber(captureDebug.postProcessModelMs, 1, " ms")}
          />
          <MetricItem
            label={t("debug.capture.metrics.residual-warp")}
            value={formatNumber(captureDebug.postProcessResidualWarpMs, 1, " ms")}
          />
          <MetricItem
            label={t("debug.capture.metrics.flatten")}
            value={formatNumber(captureDebug.postProcessFlattenMs, 1, " ms")}
          />

          <MetricItem
            label={t("debug.capture.metrics.enhance")}
            value={formatNumber(captureDebug.postProcessEnhanceMs, 1, " ms")}
          />
          <MetricItem
            label={t("debug.capture.metrics.encode")}
            value={formatNumber(captureDebug.postProcessEncodeMs, 1, " ms")}
          />

          <MetricItem
            label={t("debug.capture.metrics.input-resolution")}
            value={formatResolution(captureDebug.postProcessInputWidth, captureDebug.postProcessInputHeight)}
          />
          <MetricItem
            label={t("debug.capture.metrics.output-resolution")}
            value={formatResolution(captureDebug.postProcessOutputWidth, captureDebug.postProcessOutputHeight)}
          />
          <MetricItem
            label={t("debug.capture.metrics.last-update")}
            value={formatTimestamp(captureDebug.postProcessUpdatedAt)}
          />
        </div>

        <Separator />

        <div className="grid gap-3 sm:grid-cols-2">
          <MetricItem
            label={t("debug.capture.metrics.backend")}
            value={captureDebug.postProcessBackend ?? "—"}
            hint={captureDebug.postProcessFallbackReason ? t("debug.capture.metrics.fallback") + ": " + captureDebug.postProcessFallbackReason : undefined}
          />
          <MetricItem
            label={t("debug.capture.metrics.model-id")}
            value={captureDebug.postProcessModelId ?? "—"}
            hint={captureDebug.postProcessControlGridShape ? t("debug.capture.metrics.grid-shape") + ": " + captureDebug.postProcessControlGridShape : undefined}
          />
        </div>

        {captureDebug.postProcessError ? (
          <>
            <Separator />
            <MetricItem
              label={t("debug.capture.metrics.error")}
              value={captureDebug.postProcessError}
            />
          </>
        ) : null}
      </CardContent>
    </Card>
  );
}

export function ScannerDebugPanel() {
  return (
    <div className="flex min-w-0 w-full flex-col gap-4">
      <ScannerDetectionDebugCard />
      <ScannerPostProcessDetailsCard />
    </div>
  );
}
