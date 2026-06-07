"use client";

import dynamic from "next/dynamic";

import type {
  AppTarget,
  PlatformCaptureActionsProps,
} from "../shared/platform-types";

export type {AppTarget, PlatformCaptureActionsProps};

export const PlatformCaptureActions = dynamic(
  () => import("./components/capture/DesktopCaptureActions").then((module) => module.DesktopCaptureActions),
  {ssr: false},
);

export const PlatformInitPage = dynamic(
  () => import("@/components/init/InitWizard"),
  {ssr: false},
);

export const PlatformScannerSetupStep = dynamic(
  () => import("./components/init/ScannerSetupStep"),
  {ssr: false},
);

export const PlatformScannerSettingsCard = dynamic(
  () => import("./components/settings/ScannerSettingsCard"),
  {ssr: false},
);

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

export { default as PlatformInitGuard } from "@/components/guards/RequireInit";
