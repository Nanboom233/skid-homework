"use client";

import {useEffect} from "react";
import {useTranslation} from "react-i18next";
import {CheckCircle2, Download, FolderOpen, AlertTriangle, Loader2, X} from "lucide-react";
import {Button} from "@/components/ui/button";
import {scannerErrorI18nKey} from "../../lib/tauri/scanner";
import {useScannerStore} from "../../store/scanner-store";

export default function ScannerSetupStep() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  const assetsStatus = useScannerStore((s) => s.assetsStatus);
  const progress = useScannerStore((s) => s.progress);
  const isOperating = useScannerStore((s) => s.isOperating);
  const activeOperation = useScannerStore((s) => s.activeOperation);
  const operationError = useScannerStore((s) => s.operationError);
  const canRetryLastOperation = useScannerStore((s) => s.canRetryLastOperation);
  const canCancelCurrentOperation = useScannerStore((s) => s.canCancelCurrentOperation);
  const fetchStatus = useScannerStore((s) => s.fetchStatus);
  const startDownload = useScannerStore((s) => s.startDownload);
  const cancelCurrentOperation = useScannerStore((s) => s.cancelCurrentOperation);
  const retryLastOperation = useScannerStore((s) => s.retryLastOperation);
  const clearError = useScannerStore((s) => s.clearError);

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  useEffect(() => {
    return () => {
      const state = useScannerStore.getState();
      if (state.activeOperation?.kind === "download") {
        void state.cancelCurrentOperation({silent: true});
      }
    };
  }, []);

  const handleImport = async () => {
    const {open} = await import("@tauri-apps/plugin-dialog");
    const isWindows = navigator.userAgent.includes("Windows");
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: t("import.filter-label"),
          extensions: isWindows ? ["zip"] : ["tar.gz"],
        },
      ],
    });
    if (selected) {
      const {startImport} = useScannerStore.getState();
      await startImport(selected);
    }
  };

  const state = assetsStatus?.state ?? "missing";
  const isReady = state === "ready";
  const isMissing = state === "missing" || state === "invalid";

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold">{t("init.title")}</h2>
        <p className="mt-2 text-sm text-muted-foreground">{t("init.description")}</p>
      </div>

      {isReady && (
        <div className="flex items-center gap-3 rounded-lg border border-green-200 bg-green-50 p-4 dark:border-green-800 dark:bg-green-950">
          <CheckCircle2 className="h-5 w-5 text-green-600 dark:text-green-400" />
          <div>
            <p className="font-medium text-green-800 dark:text-green-200">{t("status.ready")}</p>
            {assetsStatus?.manifest && (
              <p className="text-sm text-green-600 dark:text-green-400">
                {t("status.version", {version: assetsStatus.manifest.assetVersion})}
              </p>
            )}
          </div>
        </div>
      )}

      {isMissing && !isOperating && !operationError && (
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">{t("init.missing-description")}</p>
          <div className="flex gap-3">
            <Button onClick={() => void startDownload()} disabled={isOperating}>
              <Download className="mr-2 h-4 w-4" />
              {t("actions.download")}
            </Button>
            <Button variant="outline" onClick={() => void handleImport()} disabled={isOperating}>
              <FolderOpen className="mr-2 h-4 w-4" />
              {t("actions.import")}
            </Button>
          </div>
        </div>
      )}

      {isOperating && (progress || activeOperation) && (
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span className="text-sm font-medium">
                {progress
                  ? t(`phases.${progress.phase}`)
                  : activeOperation?.kind === "download"
                    ? t("status.downloading")
                    : activeOperation?.kind === "clear"
                      ? t("status.clearing")
                      : t("status.importing")}
              </span>
            </div>
            {canCancelCurrentOperation && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void cancelCurrentOperation()}
              >
                <X className="mr-2 h-4 w-4" />
                {t("actions.cancel")}
              </Button>
            )}
          </div>
          {progress?.bytesTotal != null && progress.bytesTotal > 0 && (
            <div className="space-y-1">
              <div className="h-2 w-full overflow-hidden rounded-full bg-secondary">
                <div
                  className="h-full bg-primary transition-all duration-300"
                  style={{width: `${Math.min(100, ((progress.bytesDone ?? 0) / progress.bytesTotal) * 100)}%`}}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {formatBytes(progress.bytesDone ?? 0)} / {formatBytes(progress.bytesTotal)}
              </p>
            </div>
          )}
        </div>
      )}

      {operationError && (
        <div className="space-y-3 rounded-lg border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 text-red-600 dark:text-red-400" />
            <div className="flex-1">
              <p className="text-sm font-medium text-red-800 dark:text-red-200">
                {t("error.title")}
              </p>
              <p className="mt-1 text-xs text-red-600 dark:text-red-400">
                {scannerErrorI18nKey(operationError.code)
                  ? t(scannerErrorI18nKey(operationError.code)!)
                  : t("error.unknown", {code: operationError.code})}
              </p>
              <p className="mt-1 text-xs text-red-600/80 dark:text-red-400/80">
                {t("error.diagnostic-code", {code: operationError.code})}
              </p>
              {operationError.details && (
                <p className="mt-1 text-xs text-red-600/80 dark:text-red-400/80">
                  {t("error.diagnostic-details", {details: operationError.details})}
                </p>
              )}
            </div>
          </div>
          <div className="flex gap-2">
            {operationError.retryable && canRetryLastOperation && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void retryLastOperation()}
                disabled={isOperating}
              >
                {t("actions.retry")}
              </Button>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={clearError}
              disabled={isOperating}
            >
              <X className="mr-2 h-4 w-4" />
              {t("actions.cancel")}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
