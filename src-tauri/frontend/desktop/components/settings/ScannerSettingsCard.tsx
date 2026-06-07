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
} from "lucide-react";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from "@/components/ui/card";
import {Collapsible, CollapsibleContent, CollapsibleTrigger} from "@/components/ui/collapsible";
import {Badge} from "@/components/ui/badge";
import {ScannerOperationPanel} from "../scanner/ScannerOperationPanel";
import {useScannerArchiveImport} from "../scanner/useScannerArchiveImport";
import {useScannerStore} from "../../store/scanner-store";

export default function ScannerSettingsCard() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  const assetsStatus = useScannerStore((s) => s.assetsStatus);
  const probeStatus = useScannerStore((s) => s.probeStatus);
  const progress = useScannerStore((s) => s.progress);
  const isOperating = useScannerStore((s) => s.isOperating);
  const activeOperation = useScannerStore((s) => s.activeOperation);
  const operationError = useScannerStore((s) => s.operationError);
  const canRetryLastOperation = useScannerStore((s) => s.canRetryLastOperation);
  const canCancelCurrentOperation = useScannerStore((s) => s.canCancelCurrentOperation);
  const updateCheckResult = useScannerStore((s) => s.updateCheckResult);
  const fetchStatus = useScannerStore((s) => s.fetchStatus);
  const fetchProbe = useScannerStore((s) => s.fetchProbe);
  const startUpdateDownload = useScannerStore((s) => s.startUpdateDownload);
  const checkForAssetUpdate = useScannerStore((s) => s.checkForAssetUpdate);
  const clearInstalledAssets = useScannerStore((s) => s.clearInstalledAssets);
  const cancelCurrentOperation = useScannerStore((s) => s.cancelCurrentOperation);
  const retryLastOperation = useScannerStore((s) => s.retryLastOperation);
  const clearError = useScannerStore((s) => s.clearError);
  const clearUpdateCheckResult = useScannerStore((s) => s.clearUpdateCheckResult);
  const [confirmClear, setConfirmClear] = useState(false);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const importArchive = useScannerArchiveImport();

  useEffect(() => {
    void fetchStatus();
    void fetchProbe();
  }, [fetchStatus, fetchProbe]);

  useEffect(() => {
    return () => {
      const state = useScannerStore.getState();
      if (state.activeOperation?.kind === "download") {
        void state.cancelCurrentOperation();
      }
    };
  }, []);

  useEffect(() => {
    if (updateCheckResult?.kind !== "up-to-date") return;
    const timeout = window.setTimeout(clearUpdateCheckResult, 2500);
    return () => window.clearTimeout(timeout);
  }, [clearUpdateCheckResult, updateCheckResult?.kind]);

  useEffect(() => {
    if (!confirmClear) return;
    const timeout = window.setTimeout(() => setConfirmClear(false), 3000);
    return () => window.clearTimeout(timeout);
  }, [confirmClear]);

  useEffect(() => {
    if (isOperating) {
      setConfirmClear(false);
    }
  }, [isOperating]);

  const state = assetsStatus?.state ?? "missing";
  const hasDownloadedAssets = state === "ready" || state === "invalid";
  const providerStatus = state === "ready" && assetsStatus?.manifest ? probeStatus : null;
  const canStartCheckedDownload =
    updateCheckResult?.kind === "update-available" ||
    updateCheckResult?.kind === "install-available";
  const updateButtonIsGreen = canStartCheckedDownload;
  const updateButtonDisabled =
    isOperating || isCheckingUpdate || updateCheckResult?.kind === "up-to-date";
  const updateButtonLabel =
    updateCheckResult?.kind === "up-to-date"
      ? t("actions.up-to-date")
      : updateCheckResult?.kind === "update-available"
        ? t("actions.update-to", {version: updateCheckResult.version})
        : updateCheckResult?.kind === "install-available"
          ? t("actions.install-version", {version: updateCheckResult.version})
          : t("actions.check-update");

  const handleUpdateButton = async () => {
    if (canStartCheckedDownload) {
      clearUpdateCheckResult();
      await startUpdateDownload();
      return;
    }
    setIsCheckingUpdate(true);
    try {
      await checkForAssetUpdate();
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  const handleClearAssets = async () => {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
    await clearInstalledAssets();
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.title")}</CardTitle>
        <CardDescription>{t("settings.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge state={state} />
            {assetsStatus?.manifest && (
              <Badge variant="secondary">{assetsStatus.manifest.assetVersion}</Badge>
            )}
            {providerStatus && (
              <Badge variant={providerStatus.preferredProviderReady ? "default" : "secondary"}>
                {providerStatus.preferredProviderReady ? (
                  <><Zap className="mr-1 h-3 w-3" />{providerStatus.preferredProvider}</>
                ) : (
                  <><Cpu className="mr-1 h-3 w-3" />{t("status.cpu-fallback")}</>
                )}
              </Badge>
            )}
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            onClick={() => void handleUpdateButton()}
            disabled={updateButtonDisabled}
            className={
              updateButtonIsGreen
                ? "bg-green-600 text-white hover:bg-green-700"
                : undefined
            }
          >
            {canStartCheckedDownload ? (
              <Download className="mr-2 h-4 w-4" />
            ) : (
              <RefreshCw className={`mr-2 h-4 w-4 ${isCheckingUpdate ? "animate-spin" : ""}`} />
            )}
            {updateButtonLabel}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void importArchive()}
            disabled={isOperating}
          >
            <FolderOpen className="mr-2 h-4 w-4" />
            {t("actions.import")}
          </Button>
          {hasDownloadedAssets && (
            <Button
              size="sm"
              variant={confirmClear ? "destructive" : "outline"}
              onClick={() => void handleClearAssets()}
              disabled={isOperating}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              {confirmClear ? t("actions.confirm-clear") : t("actions.clear")}
            </Button>
          )}
        </div>

        <ScannerOperationPanel
          progress={progress}
          activeOperation={activeOperation}
          operationError={operationError}
          canRetryLastOperation={canRetryLastOperation}
          canCancelCurrentOperation={canCancelCurrentOperation}
          isOperating={isOperating}
          errorClassName="space-y-2 rounded-lg border border-red-200 bg-red-50 p-3 dark:border-red-800 dark:bg-red-950"
          onCancel={() => void cancelCurrentOperation()}
          onRetry={() => void retryLastOperation()}
          onClearError={clearError}
        />

        {probeStatus && (
          <Collapsible>
            <CollapsibleTrigger className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors">
              {t("diagnostics.title")}
            </CollapsibleTrigger>
            <CollapsibleContent className="mt-3 space-y-3">
              <DiagnosticsSection probeStatus={probeStatus} />
            </CollapsibleContent>
          </Collapsible>
        )}
      </CardContent>
    </Card>
  );
}

function StatusBadge({state}: {state: string}) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner.status"});
  switch (state) {
    case "ready":
      return (
        <Badge variant="default" className="bg-green-600">
          <CheckCircle2 className="mr-1 h-3 w-3" />{t("ready")}
        </Badge>
      );
    case "downloading":
    case "importing":
    case "clearing":
      return (
        <Badge variant="secondary">
          <Loader2 className="mr-1 h-3 w-3 animate-spin" />{t(state)}
        </Badge>
      );
    case "invalid":
      return (
        <Badge variant="destructive">
          <XCircle className="mr-1 h-3 w-3" />{t("invalid")}
        </Badge>
      );
    default:
      return (
        <Badge variant="outline">
          <XCircle className="mr-1 h-3 w-3" />{t("missing")}
        </Badge>
      );
  }
}

function DiagnosticsSection({probeStatus}: {probeStatus: NonNullable<ReturnType<typeof useScannerStore.getState>["probeStatus"]>}) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner.diagnostics"});
  const hasExplicitRuntimePath =
    Boolean(probeStatus.selectedRuntimeLibraryPath) ||
    Boolean(probeStatus.loadedRuntimeLibraryPath);

  return (
    <div className="space-y-2 text-xs">
      <DiagRow label={t("resource-base-dir")} value={normalizeDisplayPath(probeStatus.resourceBaseDir)} />
      <DiagRow
        label={t("selected-runtime-path")}
        value={normalizeDisplayPath(probeStatus.selectedRuntimeLibraryPath)}
      />
      <DiagRow
        label={t("loaded-runtime-path")}
        value={normalizeDisplayPath(probeStatus.loadedRuntimeLibraryPath)}
      />
      {!hasExplicitRuntimePath && (
        <DiagRow
          label={t("compat-runtime-path")}
          value={normalizeDisplayPath(probeStatus.runtimeLibraryPath)}
        />
      )}
      <DiagRow label={t("build-info")} value={probeStatus.ortBuildInfo} />
      <DiagRow label={t("providers")} value={probeStatus.availableProviders.join(", ")} />
      {probeStatus.runtimeError && (
        <DiagRow label={t("runtime-error")} value={probeStatus.runtimeError} error />
      )}

      <div className="pt-1">
        <p className="font-medium text-muted-foreground">{t("models")}</p>
        <div className="mt-1 space-y-1">
          {probeStatus.models.map((model) => (
            <div key={model.id} className="flex items-center gap-2">
              {model.sessionReady ? (
                <CheckCircle2 className="h-3 w-3 text-green-600 dark:text-green-400" />
              ) : (
                <XCircle className="h-3 w-3 text-red-600 dark:text-red-400" />
              )}
              <span className="font-mono">{model.id}</span>
              {model.sessionError && (
                <span className="text-red-600 dark:text-red-400">— {model.sessionError}</span>
              )}
            </div>
          ))}
        </div>
      </div>

      <div className="pt-1">
        <p className="font-medium text-muted-foreground">{t("resources")}</p>
        <div className="mt-1 space-y-1">
          {probeStatus.resources.map((res) => (
            <div key={res.relativePath} className="flex items-center gap-2">
              {res.exists ? (
                <CheckCircle2 className="h-3 w-3 text-green-600 dark:text-green-400" />
              ) : (
                <XCircle className="h-3 w-3 text-red-600 dark:text-red-400" />
              )}
              <span className="font-mono">{res.relativePath}</span>
            </div>
          ))}
        </div>
      </div>
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
