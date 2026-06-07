"use client";

import {useEffect} from "react";
import {useTranslation} from "react-i18next";
import {CheckCircle2, Download, FolderOpen} from "lucide-react";
import {Button} from "@/components/ui/button";
import {ScannerOperationPanel} from "../scanner/ScannerOperationPanel";
import {useScannerArchiveImport} from "../scanner/useScannerArchiveImport";
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
  const importArchive = useScannerArchiveImport();

  useEffect(() => {
    void fetchStatus();
  }, [fetchStatus]);

  useEffect(() => {
    return () => {
      const state = useScannerStore.getState();
      if (state.activeOperation?.kind === "download") {
        void state.cancelCurrentOperation();
      }
    };
  }, []);

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
            <Button variant="outline" onClick={() => void importArchive()} disabled={isOperating}>
              <FolderOpen className="mr-2 h-4 w-4" />
              {t("actions.import")}
            </Button>
          </div>
        </div>
      )}

      <ScannerOperationPanel
        progress={progress}
        activeOperation={activeOperation}
        operationError={operationError}
        canRetryLastOperation={canRetryLastOperation}
        canCancelCurrentOperation={canCancelCurrentOperation}
        isOperating={isOperating}
        onCancel={() => void cancelCurrentOperation()}
        onRetry={() => void retryLastOperation()}
        onClearError={clearError}
      />
    </div>
  );
}
