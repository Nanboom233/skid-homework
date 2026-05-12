import type {
  AppTarget,
  PlatformCaptureActionsProps,
} from "../../src-tauri/frontend/shared/platform-types";

export type {AppTarget, PlatformCaptureActionsProps};

export const APP_TARGET: AppTarget = "web";

export const isWebTarget = true;
export const isTauriDesktopTarget = false;
export const isTauriAndroidTarget = false;

export function shouldEnableSerwist(): boolean {
  return true;
}

export async function openExternalUrl(url: string): Promise<void> {
  window.open(url, "_blank", "noopener,noreferrer");
}

export function PlatformCaptureActions(
  _props: PlatformCaptureActionsProps,
): React.JSX.Element | null {
  void _props;
  return null;
}
