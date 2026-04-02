"use client";

import {Camera, RotateCw, ScanLine, Square, Waves} from "lucide-react";
import {useTranslation} from "react-i18next";

import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@/components/ui/card";
import {Label} from "@/components/ui/label";
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from "@/components/ui/select";
import {Separator} from "@/components/ui/separator";
import {Switch} from "@/components/ui/switch";
import type {ScannerDetectionBackend} from "@/store/settings-store";

interface ScannerControlsProps {
  isConnecting: boolean;
  isStreaming: boolean;
  isProcessing: boolean;
  autoCapture: boolean;
  isStable: boolean;
  requestedBackend: ScannerDetectionBackend;
  activeBackend: ScannerDetectionBackend;
  backendReady: boolean;
  nativeBackendSupported: boolean;
  nativeStrictMode: boolean;
  backendStatusMessage: string | null;
  preferredProvider: string | null;
  preferredProviderReady: boolean;
  selectedModelKind: string | null;
  selectedModelId: string | null;
  previewOrientation: "landscape" | "portrait";
  previewResolution: string;
  reconnectState: string;
  onDetectionBackendChange: (backend: ScannerDetectionBackend) => void;
  onNativeStrictModeChange: (enabled: boolean) => void;
  onAutoCaptureChange: (enabled: boolean) => void;
  onPreviewOrientationToggle: () => void;
  onStart: () => void;
  onStop: () => void;
  onPreviewCapture: () => void;
}

const getReconnectStateLabel = (
  reconnectState: string,
  t: (
    key:
      | "states.reconnect.connected"
      | "states.reconnect.connecting"
      | "states.reconnect.error"
      | "states.reconnect.idle"
      | "states.reconnect.reconnecting"
      | "states.reconnect.stopped",
  ) => string,
): string => {
  switch (reconnectState) {
    case "connected":
    case "connecting":
    case "error":
    case "idle":
    case "reconnecting":
    case "stopped":
      return t(`states.reconnect.${reconnectState}`);
    default:
      return reconnectState;
  }
};

const translateBackend = (
  backend: ScannerDetectionBackend,
  t: (key: string) => string,
): string => {
  return t(`detection-backend.options.${backend}`);
};

const translateModelKind = (
  modelKind: string | null,
  t: (key: string) => string,
): string => {
  if (modelKind === "public-baseline" || modelKind === "planned-primary") {
    return t(`detection-backend.model-kind.${modelKind}`);
  }

  return modelKind ?? "—";
};

export function ScannerControls({
  isConnecting,
  isStreaming,
  isProcessing,
  autoCapture,
  isStable,
  requestedBackend,
  activeBackend,
  backendReady,
  nativeBackendSupported,
  nativeStrictMode,
  backendStatusMessage,
  preferredProvider,
  preferredProviderReady,
  selectedModelKind,
  selectedModelId,
  previewOrientation,
  previewResolution,
  reconnectState,
  onDetectionBackendChange,
  onNativeStrictModeChange,
  onAutoCaptureChange,
  onPreviewOrientationToggle,
  onStart,
  onStop,
  onPreviewCapture,
}: ScannerControlsProps) {
  const { t } = useTranslation("commons", { keyPrefix: "document-scanner.controls" });
  const showStopAction = isConnecting || isStreaming;
  const previewOrientationLabel = previewOrientation === "landscape"
    ? t("actions.use-portrait" as never)
    : t("actions.use-landscape" as never);

  return (
    <Card className="min-w-0 gap-0">
      <CardHeader className="pb-3">
        <div className="flex flex-wrap items-center gap-2">
          <CardTitle className="flex items-center gap-2 text-base">
            <Waves className="h-4 w-4" />
            {t("title")}
          </CardTitle>
          <Badge variant={isStreaming ? "default" : "outline"}>
            {isStreaming ? t("badges.live") : t("badges.stopped")}
          </Badge>
          <Badge variant={isStable ? "default" : "outline"}>
            {isStable ? t("badges.stable") : t("badges.unstable")}
          </Badge>
        </div>
        <CardDescription>{t("description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-2.5 md:grid-cols-2">
          <div className="min-w-0 rounded-lg border bg-background/60 p-2.5">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("metrics.preview-resolution")}
            </p>
            <p className="mt-1 break-words text-sm font-semibold">{previewResolution}</p>
          </div>
          <div className="min-w-0 rounded-lg border bg-background/60 p-2.5">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("metrics.reconnect-state")}
            </p>
            <p className="mt-1 break-words text-sm font-semibold">
              {getReconnectStateLabel(reconnectState, t)}
            </p>
          </div>
        </div>

        <div className="grid gap-3 rounded-lg border bg-background/60 p-3">
          <div className="grid gap-3 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="scanner-detection-backend">
                {t("detection-backend.label")}
              </Label>
              <Select
                value={requestedBackend}
                onValueChange={(value) => onDetectionBackendChange(value as ScannerDetectionBackend)}
                disabled={isProcessing}
              >
                <SelectTrigger id="scanner-detection-backend">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="opencv">
                    {translateBackend("opencv", t)}
                  </SelectItem>
                  <SelectItem value="native-yolo" disabled={!nativeBackendSupported}>
                    {translateBackend("native-yolo", t)}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="min-w-0 rounded-lg border bg-background p-2.5">
              <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                {t("detection-backend.active")}
              </p>
              <div className="mt-1 flex flex-wrap items-center gap-2">
                <Badge variant={backendReady ? "default" : "outline"}>
                  {translateBackend(activeBackend, t)}
                </Badge>
                <Badge variant={backendReady ? "secondary" : "destructive"}>
                  {backendReady
                    ? t("detection-backend.status.ready")
                    : t("detection-backend.status.not-ready")}
                </Badge>
                {preferredProvider ? (
                  <Badge variant={preferredProviderReady ? "secondary" : "outline"}>
                    {preferredProvider}
                  </Badge>
                ) : null}
              </div>
              <p className="mt-2 text-xs text-muted-foreground">
                {backendStatusMessage ?? t("detection-backend.no-status")}
              </p>
            </div>
          </div>

          <div className="grid gap-3 md:grid-cols-2">
            <div className="min-w-0 rounded-lg border bg-background p-2.5">
              <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                {t("detection-backend.stage1-model")}
              </p>
              <p className="mt-1 break-words text-sm font-semibold">
                {selectedModelId ?? t("detection-backend.model-missing")}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {translateModelKind(selectedModelKind, t)}
              </p>
            </div>

            <div className="flex items-start space-x-3 rounded-lg border bg-background p-2.5 shadow-sm">
              <Switch
                id="scanner-native-strict-mode"
                checked={nativeStrictMode}
                onCheckedChange={onNativeStrictModeChange}
                disabled={requestedBackend !== "native-yolo" || isProcessing}
              />
              <div className="min-w-0 space-y-1">
                <Label htmlFor="scanner-native-strict-mode" className="cursor-pointer text-sm font-medium">
                  {t("detection-backend.strict-mode.label")}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {requestedBackend === "native-yolo"
                    ? t("detection-backend.strict-mode.description")
                    : t("detection-backend.strict-mode.disabled-hint")}
                </p>
              </div>
            </div>
          </div>
        </div>

        <div className={isStreaming ? "grid gap-2 sm:grid-cols-2 xl:grid-cols-3" : "grid gap-2 sm:grid-cols-2"}>
          <Button
            variant="outline"
            onClick={onPreviewOrientationToggle}
            className="w-full justify-center gap-2"
            disabled={isProcessing}
          >
            <RotateCw className="h-4 w-4" />
            {previewOrientationLabel}
          </Button>
          {!showStopAction ? (
            <Button
              onClick={onStart}
              disabled={isProcessing}
              className="w-full justify-center gap-2"
            >
              <Camera className="h-4 w-4" />
              {t("actions.start")}
            </Button>
          ) : (
            <>
              <Button
                variant="destructive"
                onClick={onStop}
                className="w-full justify-center gap-2"
                disabled={isProcessing}
              >
                <Square className="h-4 w-4" />
                {t("actions.stop")}
              </Button>
              <Button
                variant="outline"
                onClick={onPreviewCapture}
                className="w-full justify-center gap-2"
                disabled={isProcessing}
              >
                <ScanLine className="h-4 w-4" />
                {isProcessing ? t("actions.processing") : t("actions.capture")}
              </Button>
            </>
          )}
        </div>

        <Separator />

        <div className="flex items-start space-x-3 rounded-md border p-2.5 shadow-sm">
          <Switch
            id="auto-capture"
            checked={autoCapture}
            onCheckedChange={onAutoCaptureChange}
            disabled={!isStreaming || isProcessing}
          />
          <div className="min-w-0 space-y-1">
            <Label htmlFor="auto-capture" className="cursor-pointer text-sm font-medium">
              {t("auto-capture.label")}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t("auto-capture.description")}
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
