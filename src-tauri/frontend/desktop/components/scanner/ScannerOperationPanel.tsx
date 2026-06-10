"use client";

import type {TFunction} from "i18next";
import {AlertTriangle, Loader2, X} from "lucide-react";
import {useTranslation} from "react-i18next";
import {Button} from "@/components/ui/button";
import type {
  ScannerAssetsError,
  ScannerAssetsProgress,
} from "../../lib/tauri/scanner";
import {scannerErrorI18nKey} from "../../lib/tauri/scanner";
import type {ScannerOperationContext} from "../../store/scanner-store";

interface ScannerOperationPanelProps {
  progress: ScannerAssetsProgress | null;
  activeOperation: ScannerOperationContext | null;
  operationError: ScannerAssetsError | null;
  canRetryLastOperation: boolean;
  canCancelCurrentOperation: boolean;
  isOperating: boolean;
  errorClassName?: string;
  onCancel: () => void;
  onRetry: () => void;
  onClearError: () => void;
}

export function ScannerOperationPanel({
  progress,
  activeOperation,
  operationError,
  canRetryLastOperation,
  canCancelCurrentOperation,
  isOperating,
  errorClassName = "space-y-3 rounded-lg border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950",
  onCancel,
  onRetry,
  onClearError,
}: ScannerOperationPanelProps) {
  const {t, i18n} = useTranslation("commons", {keyPrefix: "scanner"});
  const showProgress = isOperating && (progress || activeOperation);

  return (
    <>
      {showProgress && (
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span className="text-sm font-medium">
                {operationPhaseLabel(t, progress, activeOperation)}
              </span>
            </div>
            {canCancelCurrentOperation && (
              <Button size="sm" variant="outline" onClick={onCancel}>
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
        <div className={errorClassName}>
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 text-red-600 dark:text-red-400" />
            <div className="flex-1">
              <p className="text-sm font-medium text-red-800 dark:text-red-200">
                {t("error.title")}
              </p>
              <p className="mt-1 text-xs text-red-600 dark:text-red-400">
                {scannerErrorMessage(t, i18n.exists.bind(i18n), operationError.code)}
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
                onClick={onRetry}
                disabled={isOperating}
              >
                {t("actions.retry")}
              </Button>
            )}
            <Button size="sm" variant="outline" onClick={onClearError} disabled={isOperating}>
              <X className="mr-2 h-4 w-4" />
              {t("actions.cancel")}
            </Button>
          </div>
        </div>
      )}
    </>
  );
}

function operationPhaseLabel(
  t: TFunction<"commons", "scanner">,
  progress: ScannerAssetsProgress | null,
  activeOperation: ScannerOperationContext | null,
) {
  if (progress) {
    return t(`phases.${progress.phase}`);
  }
  if (activeOperation?.kind === "download") {
    return t("status.downloading");
  }
  if (activeOperation?.kind === "clear") {
    return t("status.clearing");
  }
  return t("status.importing");
}

function scannerErrorMessage(
  t: TFunction<"commons", "scanner">,
  exists: (key: string) => boolean,
  code: string,
) {
  const key = scannerErrorI18nKey(code);
  if (exists(`scanner.${key}`)) {
    return tDynamic(t, key);
  }
  return t("error.unknown", {code});
}

function tDynamic(t: TFunction<"commons", "scanner">, key: string): string {
  return (t as (key: string) => string)(key);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
