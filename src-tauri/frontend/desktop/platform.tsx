"use client";

import {MoreVertical} from "lucide-react";
import Image from "next/image";
import {useCallback, useEffect, useState} from "react";
import {useTranslation} from "react-i18next";
import {toast} from "sonner";

import type {
  AppTarget,
  PlatformCaptureActionsProps,
} from "../shared/platform-types";
import {AdbRemoteConnectDialog} from "./components/dialogs/adb-remote-connect-dialog";
import {
  captureAdbScreenshot,
  connectRemoteAdbDevice,
  getSelectedDesktopAdbSerial,
  isAdbDeviceConnected,
  pairRemoteAdbDevice,
  selectDesktopAdbDevice,
} from "./lib/webadb/screenshot";
import {UnsupportedEnvironmentError} from "./lib/webadb/manager";
import {ShortcutHint} from "@/components/ShortcutHint";
import {Button} from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {useShortcut} from "@/hooks/use-shortcut";
import {TimeoutError, withTimeout} from "@/utils/timeout";

export type {AppTarget, PlatformCaptureActionsProps};

export const APP_TARGET: AppTarget = "tauri-desktop";

export const isWebTarget = false;
export const isTauriDesktopTarget = true;
export const isTauriAndroidTarget = false;

export function shouldEnableSerwist(): boolean {
  return false;
}

export async function openExternalUrl(url: string): Promise<void> {
  const {openUrl} = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

export function PlatformCaptureActions({
  appendFiles,
  disabled,
  isCompact,
}: PlatformCaptureActionsProps): React.JSX.Element | null {
  const {t} = useTranslation("commons", {keyPrefix: "upload-area"});
  const [adbBusy, setAdbBusy] = useState(false);
  const [adbBusyMode, setAdbBusyMode] = useState<"connect" | "capture" | null>(
    null,
  );
  const [adbConnected, setAdbConnected] = useState(false);
  const [adbRemoteDialogOpen, setAdbRemoteDialogOpen] = useState(false);
  const [selectedAdbSerial, setSelectedAdbSerial] = useState<string | null>(
    null,
  );

  const isDisabled = disabled || adbBusy;

  const handleAdbError = useCallback(
    (error: unknown) => {
      if (error instanceof UnsupportedEnvironmentError) {
        toast.error(t("toasts.webusb-not-supported"));
        return;
      }

      const errorMessage = error instanceof Error ? error.message : String(error);
      toast.error(t("toasts.adb-failed", {error: errorMessage}));
    },
    [t],
  );

  const refreshAdbStatus = useCallback(async (): Promise<boolean> => {
    try {
      const connected = await isAdbDeviceConnected();
      setAdbConnected(connected);
      setSelectedAdbSerial(connected ? getSelectedDesktopAdbSerial() ?? null : null);
      return connected;
    } catch (error) {
      console.error("ADB status check failed", error);
      setAdbConnected(false);
      setSelectedAdbSerial(null);
      return false;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    const updateAdbStatus = async () => {
      const connected = await refreshAdbStatus();
      if (cancelled) return;
      if (!connected) {
        setSelectedAdbSerial(null);
      }
    };

    void updateAdbStatus();

    const handleWindowFocus = () => {
      void updateAdbStatus();
    };

    window.addEventListener("focus", handleWindowFocus);

    return () => {
      cancelled = true;
      window.removeEventListener("focus", handleWindowFocus);
    };
  }, [refreshAdbStatus]);

  const handleAdbReconnect = useCallback(async () => {
    if (isDisabled) return;
    setAdbRemoteDialogOpen(true);
  }, [isDisabled]);

  const handleAdbBtnClicked = useCallback(async () => {
    if (isDisabled) return;
    const connected = await refreshAdbStatus();

    if (!connected) {
      setAdbRemoteDialogOpen(true);
      return;
    }

    try {
      setAdbBusy(true);
      setAdbBusyMode("capture");
      const file = await withTimeout(captureAdbScreenshot(), 5_000);
      appendFiles([file], "adb");
    } catch (err) {
      if (err instanceof TimeoutError) {
        toast.error(t("adb.capture-timeout"));
      } else {
        handleAdbError(err);
      }
    } finally {
      setAdbBusy(false);
      setAdbBusyMode(null);
    }
  }, [appendFiles, handleAdbError, isDisabled, refreshAdbStatus, t]);

  const handleTauriRemoteConnect = useCallback(
    async (address: string) => {
      if (isDisabled) return;
      try {
        setAdbBusy(true);
        setAdbBusyMode("connect");
        const serial = await connectRemoteAdbDevice(address);
        setAdbConnected(true);
        setSelectedAdbSerial(serial);
        setAdbRemoteDialogOpen(false);
        toast.success(t("adb.connected", {serial}));
      } catch (error) {
        handleAdbError(error);
      } finally {
        setAdbBusy(false);
        setAdbBusyMode(null);
      }
    },
    [handleAdbError, isDisabled, t],
  );

  const handleTauriPairAndConnect = useCallback(
    async (request: {pairingAddress: string; pairingCode: string}) => {
      if (isDisabled) return;
      try {
        setAdbBusy(true);
        setAdbBusyMode("connect");
        await pairRemoteAdbDevice(request);
        toast.success(t("adb.paired"));
      } catch (error) {
        handleAdbError(error);
      } finally {
        setAdbBusy(false);
        setAdbBusyMode(null);
      }
    },
    [handleAdbError, isDisabled, t],
  );

  const handleTauriDeviceSelect = useCallback(
    async (serial: string) => {
      if (isDisabled) return;
      try {
        setAdbBusy(true);
        setAdbBusyMode("connect");
        const selectedSerial = await selectDesktopAdbDevice(serial);
        setAdbConnected(true);
        setSelectedAdbSerial(selectedSerial);
        setAdbRemoteDialogOpen(false);
        toast.success(t("adb.connected", {serial: selectedSerial}));
      } catch (error) {
        handleAdbError(error);
      } finally {
        setAdbBusy(false);
        setAdbBusyMode(null);
      }
    },
    [handleAdbError, isDisabled, t],
  );

  const adbScreenshotShortcut = useShortcut(
    "adbScreenshot",
    () => {
      void handleAdbBtnClicked();
    },
    [handleAdbBtnClicked],
  );

  if (isCompact) {
    return null;
  }

  return (
    <>
      <div className="flex gap-2">
        <Button
          variant="outline"
          className="flex-1 items-center min-w-0 justify-between"
          size="default"
          disabled={isDisabled}
          onClick={() => void handleAdbBtnClicked()}
          title={t("adb.screenshot-hint")}
        >
          <span className="flex items-center gap-1.5 min-w-0">
            <Image
              src="/icons/adb.svg"
              alt="ADB"
              width={18}
              height={18}
              className="h-4.5 w-4.5"
            />
            <span className="truncate">
              {adbBusy
                ? adbBusyMode === "capture"
                  ? t("adb.screenshot-busy")
                  : t("adb.connecting")
                : adbConnected
                  ? t("adb.screenshot")
                  : t("adb.connect")}
            </span>
          </span>
          <ShortcutHint shortcut={adbScreenshotShortcut} />
        </Button>
        {adbConnected && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="px-3"
                disabled={isDisabled}
                aria-label={t("adb.menu-aria-label")}
              >
                <MoreVertical className="h-5 w-5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-40">
              <DropdownMenuItem onClick={() => void handleAdbReconnect()}>
                {t("adb.reconnect")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      <AdbRemoteConnectDialog
        isOpen={adbRemoteDialogOpen}
        isSubmitting={adbBusy && adbBusyMode === "connect"}
        onOpenChange={setAdbRemoteDialogOpen}
        onConnect={handleTauriRemoteConnect}
        onPair={handleTauriPairAndConnect}
        onSelectDevice={handleTauriDeviceSelect}
        selectedSerial={selectedAdbSerial}
      />
    </>
  );
}

export { default as PlatformInitPage } from "@/components/init/InitWizard";
export { default as PlatformInitGuard } from "@/components/guards/RequireInit";
