"use client";

import {useEffect, useState} from "react";
import {useTranslation} from "react-i18next";
import {
  CheckCircle2,
  XCircle,
  Download,
  FolderOpen,
  Loader2,
  Cpu,
  Zap,
  RefreshCw,
  Trash2,
  File as FileIcon,
  Folder,
} from "lucide-react";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from "@/components/ui/card";
import {Collapsible, CollapsibleContent, CollapsibleTrigger} from "@/components/ui/collapsible";
import {Separator} from "@/components/ui/separator";
import {Badge} from "@/components/ui/badge";
import {createScannerAssetView, type ScannerAssetView} from "../scanner/scannerAssetView";
import {ScannerOperationPanel} from "../scanner/ScannerOperationPanel";
import {useScannerArchiveImport} from "../scanner/useScannerArchiveImport";
import type {
  ScannerOrtProbeStatus,
  ScannerOrtResourceTreeEntry,
} from "../../lib/tauri/scanner";
import {useScannerStore} from "../../store/scanner-store";

export default function ScannerSettingsCard() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  const assetsStatus = useScannerStore((s) => s.assetsStatus);
  const probeStatus = useScannerStore((s) => s.probeStatus);
  const operation = useScannerStore((s) => s.operation);
  const updateCheckResult = useScannerStore((s) => s.updateCheckResult);
  const fetchProbe = useScannerStore((s) => s.fetchProbe);
  const checkForUpdate = useScannerStore((s) => s.checkForUpdate);
  const startDownload = useScannerStore((s) => s.startDownload);
  const startUpdateAll = useScannerStore((s) => s.startUpdateAll);
  const clearAssets = useScannerStore((s) => s.clearAssets);
  const cancelCurrentOperation = useScannerStore((s) => s.cancelCurrentOperation);
  const retryLastOperation = useScannerStore((s) => s.retryLastOperation);
  const clearError = useScannerStore((s) => s.clearError);
  const [confirmClearCamera, setConfirmClearCamera] = useState(false);
  const [confirmClearOrt, setConfirmClearOrt] = useState(false);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const importOrt = useScannerArchiveImport("onnxruntime");
  const importCamera = useScannerArchiveImport("camera-server");

  useEffect(() => {
    let active = true;
    setIsCheckingUpdate(true);
    void checkForUpdate().finally(() => {
      if (active) {
        setIsCheckingUpdate(false);
      }
    });
    void fetchProbe();
    return () => {
      active = false;
    };
  }, [checkForUpdate, fetchProbe]);

  useEffect(() => {
    return () => {
      const state = useScannerStore.getState();
      if (state.operation.active?.kind === "download") {
        void state.cancelCurrentOperation();
      }
    };
  }, []);

  useEffect(() => {
    if (!confirmClearCamera) return;
    const timeout = window.setTimeout(() => setConfirmClearCamera(false), 3000);
    return () => window.clearTimeout(timeout);
  }, [confirmClearCamera]);

  useEffect(() => {
    if (!confirmClearOrt) return;
    const timeout = window.setTimeout(() => setConfirmClearOrt(false), 3000);
    return () => window.clearTimeout(timeout);
  }, [confirmClearOrt]);

  useEffect(() => {
    if (operation.isOperating) {
      setConfirmClearCamera(false);
      setConfirmClearOrt(false);
    }
  }, [operation.isOperating]);

  const cameraStatus = assetsStatus?.["camera-server"];
  const ortStatus = assetsStatus?.onnxruntime;
  const cameraUpdate = updateCheckResult?.["camera-server"];
  const ortUpdate = updateCheckResult?.onnxruntime;
  const cameraView = createScannerAssetView({
    target: "camera-server",
    status: cameraStatus,
    update: cameraUpdate,
    operation,
    fallbackErrorTarget: true,
  });
  const ortView = createScannerAssetView({
    target: "onnxruntime",
    status: ortStatus,
    update: ortUpdate,
    operation,
  });
  const anyUpdateAvailable = cameraView.updateAvailable || ortView.updateAvailable;

  const handleCheckUpdate = async () => {
    setIsCheckingUpdate(true);
    try {
      await checkForUpdate();
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const handleUpdateAll = async () => {
    await startUpdateAll();
  };

  const handleClearCamera = async () => {
    if (!confirmClearCamera) {
      setConfirmClearCamera(true);
      return;
    }
    setConfirmClearCamera(false);
    await clearAssets("camera-server");
  };

  const handleClearOrt = async () => {
    if (!confirmClearOrt) {
      setConfirmClearOrt(true);
      return;
    }
    setConfirmClearOrt(false);
    await clearAssets("onnxruntime");
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.title")}</CardTitle>
        <CardDescription>{t("settings.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap gap-2">
          {anyUpdateAvailable ? (
            <Button
              size="sm"
              className="bg-green-600 text-white hover:bg-green-700"
              onClick={() => void handleUpdateAll()}
              disabled={operation.isOperating}
            >
              <Download className="mr-2 h-4 w-4" />
              {t("actions.update-all")}
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={() => void handleCheckUpdate()}
              disabled={operation.isOperating || isCheckingUpdate}
            >
              <RefreshCw className={`mr-2 h-4 w-4 ${isCheckingUpdate ? "animate-spin" : ""}`} />
              {t("actions.check-update")}
            </Button>
          )}
        </div>

        <div className="space-y-2">
          <p className="text-sm font-medium">{t("target.camera")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge state={cameraView.state} />
            {cameraView.installedVersion && (
              <Badge variant="secondary">{cameraView.installedVersion}</Badge>
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            {(cameraView.showDownloadAction || cameraView.showUpdateAction) && (
              <Button
                size="sm"
                className={cameraView.showUpdateAction ? "bg-green-600 text-white hover:bg-green-700" : undefined}
                onClick={() => void startDownload("camera-server")}
                disabled={operation.isOperating}
              >
                <Download className="mr-2 h-4 w-4" />
                {cameraView.showUpdateAction
                  ? t("actions.update-to", {version: cameraView.targetVersion ?? ""})
                  : downloadLabel(t("actions.download"), cameraView.targetVersion)}
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() => void importCamera()}
              disabled={operation.isOperating}
            >
              <FolderOpen className="mr-2 h-4 w-4" />
              {t("actions.import")}
            </Button>
            {cameraView.hasAssets && (
              <Button
                size="sm"
                variant={confirmClearCamera ? "destructive" : "outline"}
                onClick={() => void handleClearCamera()}
                disabled={operation.isOperating}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                {confirmClearCamera ? t("actions.confirm-clear") : t("actions.clear")}
              </Button>
            )}
          </div>
          <TargetOperationPanel
            view={cameraView}
            onCancel={() => void cancelCurrentOperation()}
            onRetry={() => void retryLastOperation()}
            onClearError={clearError}
          />
        </div>

        <Separator />

        <div className="space-y-2">
          <p className="text-sm font-medium">{t("target.ort")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge state={ortView.state} />
            {ortView.installedVersion && (
              <Badge variant="secondary">{ortView.installedVersion}</Badge>
            )}
            {ortView.isReady && probeStatus && (
              <Badge variant={probeStatus.preferredProviderReady ? "default" : "secondary"}>
                {probeStatus.preferredProviderReady ? (
                  <><Zap className="mr-1 h-3 w-3" />{probeStatus.preferredProvider}</>
                ) : (
                  <><Cpu className="mr-1 h-3 w-3" />{t("status.cpu-fallback")}</>
                )}
              </Badge>
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            {(ortView.showDownloadAction || ortView.showUpdateAction) && (
              <Button
                size="sm"
                className={ortView.showUpdateAction ? "bg-green-600 text-white hover:bg-green-700" : undefined}
                onClick={() => void startDownload("onnxruntime")}
                disabled={operation.isOperating}
              >
                <Download className="mr-2 h-4 w-4" />
                {ortView.showUpdateAction
                  ? t("actions.update-to", {version: ortView.targetVersion ?? ""})
                  : downloadLabel(t("actions.download"), ortView.targetVersion)}
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() => void importOrt()}
              disabled={operation.isOperating}
            >
              <FolderOpen className="mr-2 h-4 w-4" />
              {t("actions.import")}
            </Button>
            {ortView.hasAssets && (
              <Button
                size="sm"
                variant={confirmClearOrt ? "destructive" : "outline"}
                onClick={() => void handleClearOrt()}
                disabled={operation.isOperating}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                {confirmClearOrt ? t("actions.confirm-clear") : t("actions.clear")}
              </Button>
            )}
          </div>
          <TargetOperationPanel
            view={ortView}
            onCancel={() => void cancelCurrentOperation()}
            onRetry={() => void retryLastOperation()}
            onClearError={clearError}
          />
        </div>

        {(probeStatus || cameraStatus?.artifact) && (
          <Collapsible>
            <CollapsibleTrigger className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors">
              {t("diagnostics.title")}
            </CollapsibleTrigger>
            <CollapsibleContent className="mt-3">
              <ScannerDiagnostics
                serverPath={cameraStatus?.artifact?.path}
                probeStatus={probeStatus}
              />
            </CollapsibleContent>
          </Collapsible>
        )}
      </CardContent>
    </Card>
  );
}

function downloadLabel(label: string, version?: string) {
  return version ? `${label} ${version}` : label;
}

function TargetOperationPanel({
  view,
  onCancel,
  onRetry,
  onClearError,
}: {
  view: ScannerAssetView;
  onCancel: () => void;
  onRetry: () => void;
  onClearError: () => void;
}) {
  if (!view.showPanel) return null;

  return (
    <ScannerOperationPanel
      progress={view.panelProgress}
      activeOperation={view.panelActiveOperation}
      operationError={view.panelError}
      canRetry={view.panelCanRetry}
      canCancel={view.panelCanCancel}
      isOperating={view.panelIsOperating}
      errorClassName="space-y-2 rounded-lg border border-red-200 bg-red-50 p-3 dark:border-red-800 dark:bg-red-950"
      onCancel={onCancel}
      onRetry={onRetry}
      onClearError={onClearError}
    />
  );
}

function StatusBadge({state}: {state: string}) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  switch (state) {
    case "ready":
      return (
        <Badge variant="default" className="bg-green-600">
          <CheckCircle2 className="mr-1 h-3 w-3" />{t("settings.ready")}
        </Badge>
      );
    case "downloading":
    case "importing":
    case "clearing":
      return (
        <Badge variant="secondary">
          <Loader2 className="mr-1 h-3 w-3 animate-spin" />{t(`status.${state}`)}
        </Badge>
      );
    case "invalid":
      return (
        <Badge variant="destructive">
          <XCircle className="mr-1 h-3 w-3" />{t("status.invalid")}
        </Badge>
      );
    default:
      return (
        <Badge variant="outline">
          <XCircle className="mr-1 h-3 w-3" />{t("status.missing")}
        </Badge>
      );
  }
}

function ScannerDiagnostics({
  serverPath,
  probeStatus,
}: {
  serverPath?: string;
  probeStatus: ScannerOrtProbeStatus | null;
}) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner.diagnostics"});
  const onnxRuntimePath = probeStatus
    ? normalizeDisplayPath(
        probeStatus.selectedRuntimeLibraryPath
        ?? probeStatus.loadedRuntimeLibraryPath
        ?? probeStatus.runtimeLibraryPath,
      )
    : undefined;

  return (
    <div className="space-y-3 text-xs">
      <DiagRow label={t("server-path")} value={normalizeDisplayPath(serverPath)} />
      <DiagRow label={t("onnxruntime-path")} value={onnxRuntimePath} />
      <DiagRow label={t("onnxruntime-build-info")} value={probeStatus?.ortBuildInfo} />
      <DiagRow
        label={t("onnxruntime-providers")}
        value={probeStatus?.availableProviders.join(", ")}
      />
      {probeStatus && (
        <ResourceTree resources={probeStatus.resourceTree} />
      )}
    </div>
  );
}

function DiagRow({label, value, error}: {label: string; value?: string | null; error?: boolean}) {
  if (!value) return null;
  return (
    <div className="flex gap-2">
      <span className="shrink-0 font-medium text-muted-foreground">{label}:</span>
      <span className={`break-all font-mono ${error ? "text-red-600 dark:text-red-400" : ""}`}>{value}</span>
    </div>
  );
}

function ResourceTree({
  resources,
}: {
  resources: ScannerOrtResourceTreeEntry[];
}) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner.diagnostics"});
  const fileCount = resources.filter((resource) => !resource.isDir).length;
  const directoryCount = resources.length - fileCount;
  const totalSizeBytes = resources.reduce(
    (total, resource) => resource.isDir ? total : total + (resource.sizeBytes ?? 0),
    0,
  );

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <p className="font-medium text-muted-foreground">{t("resources")}</p>
        <p className="text-muted-foreground">
          {t("resources-summary", {
            files: fileCount,
            directories: directoryCount,
            size: formatFileSize(totalSizeBytes),
          })}
        </p>
      </div>

      {resources.length > 0 ? (
        <div className="max-h-64 overflow-auto rounded-md border bg-muted/20 py-1">
          {resources.map((resource) => (
            <div
              key={resource.relativePath}
              className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-1.5 hover:bg-muted/40"
            >
              <div
                className="flex min-w-0 items-center gap-2"
                style={{paddingLeft: `${resource.depth * 16}px`}}
              >
                {resource.isDir ? (
                  <Folder className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                ) : (
                  <FileIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                )}
                <span className="truncate font-mono" title={resource.relativePath}>
                  {resource.name}
                </span>
              </div>
              <span className="shrink-0 font-mono text-muted-foreground">
                {formatFileSize(resource.sizeBytes)}
              </span>
            </div>
          ))}
        </div>
      ) : (
        <p className="rounded-md border bg-muted/20 px-3 py-2 font-mono text-muted-foreground">
          {t("empty-resource-tree")}
        </p>
      )}
    </div>
  );
}

function formatFileSize(sizeBytes?: number) {
  if (sizeBytes == null) return "-";
  if (sizeBytes < 1024) return `${sizeBytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = sizeBytes / 1024;
  for (const unit of units) {
    if (value < 1024) return `${value.toFixed(value >= 100 ? 0 : 1)} ${unit}`;
    value /= 1024;
  }
  return `${value.toFixed(1)} PB`;
}

function normalizeDisplayPath(value?: string | null) {
  const trimmed = value?.trim();
  if (!trimmed) return undefined;

  const isUnc = /^[\\/]{2}[^\\/]/.test(trimmed);
  const isDrivePath = /^[A-Za-z]:[\\/]/.test(trimmed);
  const preferredSeparator = isDrivePath || trimmed.includes("\\") ? "\\" : "/";
  let prefix = "";
  let rest = trimmed;

  if (isUnc) {
    prefix = preferredSeparator.repeat(2);
    rest = trimmed.replace(/^[\\/]+/, "");
  } else if (isDrivePath) {
    prefix = `${trimmed.slice(0, 2)}${preferredSeparator}`;
    rest = trimmed.slice(3);
  } else if (/^[\\/]/.test(trimmed)) {
    prefix = preferredSeparator;
    rest = trimmed.replace(/^[\\/]+/, "");
  }

  rest = rest
    .replace(/[\\/]+$/, "")
    .replace(/[\\/]+/g, preferredSeparator);

  return `${prefix}${rest}`;
}
