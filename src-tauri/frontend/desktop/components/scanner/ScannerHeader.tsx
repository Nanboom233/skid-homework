import {useTranslation} from "react-i18next";
import {Button} from "@/components/ui/button";
import {Badge} from "@/components/ui/badge";
import {X, Camera, RotateCw, Settings, Wifi, WifiOff, Loader2, AlertCircle} from "lucide-react";
import {cn} from "@/lib/utils";

interface ScannerHeaderProps {
  onClose: () => void;
  isEditing: boolean;
  editingLabel?: string;
  deviceStatus: "connected" | "connecting" | "disconnected" | "error";
  onToggleDiagnostics?: () => void;
  onToggleOrientation?: () => void;
  onSettingsClick?: () => void;
}

export function ScannerHeader({
  onClose,
  isEditing,
  editingLabel,
  deviceStatus,
  onToggleDiagnostics,
  onToggleOrientation,
  onSettingsClick,
}: ScannerHeaderProps) {
  const {t} = useTranslation("commons", {keyPrefix: "document-scanner"});

  const getStatusIcon = () => {
    switch (deviceStatus) {
      case "connected": return <Wifi className="h-4 w-4" />;
      case "connecting": return <Loader2 className="h-4 w-4 animate-spin" />;
      case "error": return <AlertCircle className="h-4 w-4" />;
      default: return <WifiOff className="h-4 w-4" />;
    }
  };

  const getStatusColor = () => {
    switch (deviceStatus) {
      case "connected": return "bg-green-500/15 text-green-500 hover:bg-green-500/25";
      case "connecting": return "bg-yellow-500/15 text-yellow-500 hover:bg-yellow-500/25";
      case "error": return "bg-red-500/15 text-red-500 hover:bg-red-500/25";
      default: return "bg-muted text-muted-foreground";
    }
  };

  return (
    <div className="flex h-14 shrink-0 items-center justify-between border-b px-4">
      <div className="flex items-center gap-2">
        <div className="flex items-center gap-2 font-semibold">
          <Camera className="hidden h-5 w-5 text-muted-foreground sm:inline-block" />
          <span>{isEditing ? editingLabel || t("status.editing", "Editing") : t("title", "Scanner")}</span>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <Badge variant="secondary" className={cn("gap-1.5 capitalize px-2.5 py-1", getStatusColor())}>
          {getStatusIcon()}
          <span className="sr-only lg:not-sr-only lg:inline-block">
            {t(`status.device.${deviceStatus}`, deviceStatus)}
          </span>
        </Badge>

        {onToggleOrientation && (
          <Button variant="ghost" size="icon" onClick={onToggleOrientation} className="hidden sm:flex">
            <RotateCw className="h-5 w-5" />
            <span className="sr-only">{t("actions.toggle-orientation", "Toggle orientation")}</span>
          </Button>
        )}

        {onSettingsClick && (
          <Button variant="ghost" size="icon" onClick={onSettingsClick} className="hidden sm:flex">
            <Settings className="h-5 w-5" />
            <span className="sr-only">{t("actions.settings", "Settings")}</span>
          </Button>
        )}

        {onToggleDiagnostics && (
          <Button variant="ghost" size="icon" onClick={onToggleDiagnostics}>
            <Settings className="h-5 w-5" />
            <span className="sr-only">{t("actions.diagnostics", "Diagnostics")}</span>
          </Button>
        )}

        <div className="mx-1 h-4 w-px bg-border" aria-hidden="true" />

        <Button variant="ghost" size="icon" onClick={onClose}>
          <X className="h-5 w-5" />
          <span className="sr-only">{t("actions.close", "Close")}</span>
        </Button>
      </div>
    </div>
  );
}
