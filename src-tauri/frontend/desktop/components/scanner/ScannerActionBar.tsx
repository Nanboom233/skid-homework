import {Circle, Play, Square, Zap} from "lucide-react";
import {useId} from "react";
import {useTranslation} from "react-i18next";

import {Button} from "@/components/ui/button";
import {Label} from "@/components/ui/label";
import {Switch} from "@/components/ui/switch";
import {cn} from "@/lib/utils";

interface ScannerActionBarProps {
  onCapture: () => void;
  isCapturing: boolean;
  autoCapture: boolean;
  onAutoCaptureChange: (value: boolean) => void;
  isStreaming: boolean;
  onStartStop: () => void;
  isConnecting: boolean;
  disabled?: boolean;
}

export function ScannerActionBar({
  onCapture,
  isCapturing,
  autoCapture,
  onAutoCaptureChange,
  isStreaming,
  onStartStop,
  isConnecting,
  disabled,
}: ScannerActionBarProps) {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});
  const autoCaptureId = useId();

  return (
    <div className="flex h-14 w-full items-center justify-between bg-background px-4 lg:h-16">
      <div className="flex flex-1 items-center gap-2">
        <Switch
          id={autoCaptureId}
          checked={autoCapture}
          onCheckedChange={onAutoCaptureChange}
          disabled={disabled || !isStreaming}
        />
        <Label htmlFor={autoCaptureId} className="cursor-pointer text-sm font-medium">
          <span className="flex items-center gap-1.5">
            <Zap className="h-4 w-4" />
            <span className="hidden sm:inline">{t("auto-capture", "Auto")}</span>
          </span>
        </Label>
      </div>

      <div className="flex flex-1 justify-center">
        <Button
          size="icon"
          className="h-14 w-14 rounded-full bg-primary text-primary-foreground shadow-lg hover:bg-primary/90"
          onClick={onCapture}
          disabled={disabled || !isStreaming || isCapturing}
          aria-label={t("capture", "Capture")}
        >
          <Circle className={cn("h-6 w-6", isCapturing && "animate-pulse")} />
        </Button>
      </div>

      <div className="flex flex-1 justify-end">
        <Button
          variant={isStreaming ? "destructive" : "default"}
          onClick={onStartStop}
          disabled={disabled || isConnecting}
          className="gap-2"
        >
          {isStreaming ? (
            <>
              <Square className="h-4 w-4" />
              <span className="hidden sm:inline">{t("stop", "Stop")}</span>
            </>
          ) : (
            <>
              <Play className="h-4 w-4" />
              <span className="hidden sm:inline">{t("start", "Start")}</span>
            </>
          )}
        </Button>
      </div>
    </div>
  );
}
