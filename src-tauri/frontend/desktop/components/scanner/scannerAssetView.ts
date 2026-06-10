import type {
  AssetTarget,
  CameraAssetStatus,
  CameraUpdateCheck,
  OrtAssetStatus,
  OrtUpdateCheck,
  ScannerAssetsError,
  ScannerAssetsProgress,
} from "../../lib/tauri/scanner";
import type {
  ScannerOperationContext,
  ScannerOperationState,
} from "../../store/scanner-store";

type TargetStatus = CameraAssetStatus | OrtAssetStatus | undefined;
type TargetUpdate = CameraUpdateCheck | OrtUpdateCheck | undefined;

export interface ScannerAssetView {
  target: AssetTarget;
  isBootstrapping: boolean;
  state: string;
  isReady: boolean;
  hasAssets: boolean;
  installedVersion?: string;
  targetVersion?: string;
  updateAvailable: boolean;
  showReady: boolean;
  showDownloadAction: boolean;
  showUpdateAction: boolean;
  showPanel: boolean;
  panelProgress: ScannerAssetsProgress | null;
  panelActiveOperation: ScannerOperationContext | null;
  panelError: ScannerAssetsError | null;
  panelCanRetry: boolean;
  panelCanCancel: boolean;
  panelIsOperating: boolean;
}

interface ScannerAssetViewInput {
  target: AssetTarget;
  status: TargetStatus;
  update: TargetUpdate;
  operation: ScannerOperationState;
  isBootstrapping?: boolean;
  fallbackErrorTarget?: boolean;
}

export function createScannerAssetView({
  target,
  status,
  update,
  operation,
  isBootstrapping = false,
  fallbackErrorTarget = false,
}: ScannerAssetViewInput): ScannerAssetView {
  const state = status?.state ?? "missing";
  const isReady = state === "ready";
  const hasAssets = state === "ready" || state === "invalid";
  const isMissing = state === "missing" || state === "invalid";
  const updateAvailable = Boolean(isReady && update?.updateAvailable);
  const activeTarget = targetFromOperation(operation.active);
  const retryTarget = targetFromOperation(operation.retry);
  const isActiveTarget = activeTarget === target;
  const ownsError = Boolean(
    operation.error &&
      !activeTarget &&
      (retryTarget === target || (!operation.retry && fallbackErrorTarget)),
  );
  const controlsVisible = !isBootstrapping && !ownsError;

  return {
    target,
    isBootstrapping,
    state,
    isReady,
    hasAssets,
    installedVersion: installedVersionForTarget(target, status),
    targetVersion: update?.targetAssetTag,
    updateAvailable,
    showReady: controlsVisible && isReady && Boolean(update) && !updateAvailable,
    showDownloadAction: controlsVisible && isMissing,
    showUpdateAction: controlsVisible && updateAvailable,
    showPanel: isActiveTarget || ownsError,
    panelProgress: isActiveTarget ? operation.progress : null,
    panelActiveOperation: isActiveTarget ? operation.active : null,
    panelError: ownsError ? operation.error : null,
    panelCanRetry: ownsError && operation.canRetry,
    panelCanCancel: isActiveTarget && operation.canCancel,
    panelIsOperating: isActiveTarget && operation.isOperating,
  };
}

export function targetFromOperation(
  operation: ScannerOperationContext | null,
): AssetTarget | null {
  if (!operation || !("target" in operation)) return null;
  return operation.target;
}

function installedVersionForTarget(
  target: AssetTarget,
  status: TargetStatus,
): string | undefined {
  if (!status) return undefined;
  if (target === "camera-server") {
    return (status as CameraAssetStatus).artifact?.assetVersion;
  }
  return (status as OrtAssetStatus).manifest?.assetVersion;
}
