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
import type {ScannerPostProcessBackend} from "@/store/settings-store";

interface ScannerControlsProps {
  isConnecting: boolean;
  isStreaming: boolean;
  isProcessing: boolean;
  autoCapture: boolean;
  isStable: boolean;
  requestedPostProcessBackend: ScannerPostProcessBackend;
  previewOrientation: "landscape" | "portrait";
  imageEnhancement: boolean;
  onPostProcessBackendChange: (backend: ScannerPostProcessBackend) => void;
  onAutoCaptureChange: (enabled: boolean) => void;
  onImageEnhancementChange: (enabled: boolean) => void;
  onPreviewOrientationToggle: () => void;
  onStart: () => void;
  onStop: () => void;
  onPreviewCapture: () => void;
}

const translatePostProcessBackend = (
  backend: ScannerPostProcessBackend,
  t: (key: string) => string,
): string => {
  return t(`postprocess-backend.options.${backend}`);
};

export function ScannerControls({
  isConnecting,
  isStreaming,
  isProcessing,
  autoCapture,
  isStable,
  requestedPostProcessBackend,
  previewOrientation,
  imageEnhancement,
  onPostProcessBackendChange,
  onAutoCaptureChange,
  onImageEnhancementChange,
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
        <div className="grid gap-3 rounded-lg border bg-background/60 p-3">
          <div className="grid gap-3 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="scanner-postprocess-backend">
                {t("postprocess-backend.label")}
              </Label>
              <Select
                value={requestedPostProcessBackend}
                onValueChange={(value) => onPostProcessBackendChange(value as ScannerPostProcessBackend)}
                disabled={isProcessing}
              >
                <SelectTrigger id="scanner-postprocess-backend">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="heuristic">
                    {translatePostProcessBackend("heuristic", t)}
                  </SelectItem>
                  <SelectItem value="native-ml-v1">
                    {translatePostProcessBackend("native-ml-v1", t)}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="flex flex-col justify-center">
              <p className="text-xs text-muted-foreground pt-3 md:pt-6">
                {t("postprocess-backend.description")}
              </p>
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

        <div className="grid gap-3 md:grid-cols-2">
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

          <div className="flex items-start space-x-3 rounded-md border p-2.5 shadow-sm">
            <Switch
              id="image-enhancement"
              checked={imageEnhancement}
              onCheckedChange={onImageEnhancementChange}
              disabled={isProcessing}
            />
            <div className="min-w-0 space-y-1">
              <Label htmlFor="image-enhancement" className="cursor-pointer text-sm font-medium">
                {t("image-enhancement.label")}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t("image-enhancement.description")}
              </p>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
