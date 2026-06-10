"use client";

import {useEffect, useState} from "react";
import {useTranslation} from "react-i18next";
import {CheckCircle2, Download, FolderOpen, Loader2} from "lucide-react";
import {Button} from "@/components/ui/button";
import {Separator} from "@/components/ui/separator";
import {createScannerAssetView, type ScannerAssetView} from "../scanner/scannerAssetView";
import {ScannerOperationPanel} from "../scanner/ScannerOperationPanel";
import {useScannerArchiveImport} from "../scanner/useScannerArchiveImport";
import {useScannerStore} from "../../store/scanner-store";

export default function ScannerSetupStep() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  const assetsStatus = useScannerStore((s) => s.assetsStatus);
  const operation = useScannerStore((s) => s.operation);
  const updateCheckResult = useScannerStore((s) => s.updateCheckResult);
  const checkForUpdate = useScannerStore((s) => s.checkForUpdate);
  const startDownload = useScannerStore((s) => s.startDownload);
  const cancelCurrentOperation = useScannerStore((s) => s.cancelCurrentOperation);
  const retryLastOperation = useScannerStore((s) => s.retryLastOperation);
  const clearError = useScannerStore((s) => s.clearError);
  const importOrt = useScannerArchiveImport("onnxruntime");
  const importCamera = useScannerArchiveImport("camera-server");
  const [isBootstrapping, setIsBootstrapping] = useState(true);

  useEffect(() => {
    let active = true;
    void checkForUpdate().finally(() => {
      if (active) {
        setIsBootstrapping(false);
      }
    });
    return () => {
      active = false;
    };
  }, [checkForUpdate]);

  useEffect(() => {
    return () => {
      const state = useScannerStore.getState();
      if (state.operation.active?.kind === "download") {
        void state.cancelCurrentOperation();
      }
    };
  }, []);

  const cameraView = createScannerAssetView({
    target: "camera-server",
    status: assetsStatus?.["camera-server"],
    update: updateCheckResult?.["camera-server"],
    operation,
    isBootstrapping,
    fallbackErrorTarget: true,
  });
  const ortView = createScannerAssetView({
    target: "onnxruntime",
    status: assetsStatus?.onnxruntime,
    update: updateCheckResult?.onnxruntime,
    operation,
    isBootstrapping,
  });

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl font-semibold">{t("init.title")}</h2>
        <p className="mt-2 text-sm text-muted-foreground">{t("init.description")}</p>
      </div>

      <ScannerSetupTargetSection
        title={t("target.camera")}
        view={cameraView}
        isOperating={operation.isOperating}
        onDownload={() => void startDownload("camera-server")}
        onImport={() => void importCamera()}
        onCancel={() => void cancelCurrentOperation()}
        onRetry={() => void retryLastOperation()}
        onClearError={clearError}
      />

      <Separator />

      <ScannerSetupTargetSection
        title={t("target.ort")}
        view={ortView}
        isOperating={operation.isOperating}
        onDownload={() => void startDownload("onnxruntime")}
        onImport={() => void importOrt()}
        onCancel={() => void cancelCurrentOperation()}
        onRetry={() => void retryLastOperation()}
        onClearError={clearError}
      />
    </div>
  );
}

interface ScannerSetupTargetSectionProps {
  title: string;
  view: ScannerAssetView;
  isOperating: boolean;
  onDownload: () => void;
  onImport: () => void;
  onCancel: () => void;
  onRetry: () => void;
  onClearError: () => void;
}

function ScannerSetupTargetSection({
  title,
  view,
  isOperating,
  onDownload,
  onImport,
  onCancel,
  onRetry,
  onClearError,
}: ScannerSetupTargetSectionProps) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});

  return (
    <div className="space-y-3">
      <p className="text-sm font-medium">{title}</p>
      {view.isBootstrapping && <BootstrapStatus />}
      {view.showReady && (
        <div className="flex items-center gap-3 rounded-lg border border-green-200 bg-green-50 p-3 dark:border-green-800 dark:bg-green-950">
          <CheckCircle2 className="h-5 w-5 text-green-600 dark:text-green-400" />
          <div>
            <p className="font-medium text-green-800 dark:text-green-200">{t("status.ready")}</p>
            {view.installedVersion && (
              <p className="text-sm text-green-600 dark:text-green-400">
                {t("status.version", {version: view.installedVersion})}
              </p>
            )}
          </div>
        </div>
      )}
      {(view.showUpdateAction || view.showDownloadAction) && (
        <div className="flex gap-3">
          <Button
            className={view.showUpdateAction ? "bg-green-600 text-white hover:bg-green-700" : undefined}
            onClick={onDownload}
            disabled={isOperating}
          >
            <Download className="mr-2 h-4 w-4" />
            {view.showUpdateAction
              ? t("actions.update-to", {version: view.targetVersion ?? ""})
              : downloadLabel(t("actions.download"), view.targetVersion)}
          </Button>
          <Button variant="outline" onClick={onImport} disabled={isOperating}>
            <FolderOpen className="mr-2 h-4 w-4" />
            {t("actions.import")}
          </Button>
        </div>
      )}
      {view.showPanel && (
        <ScannerOperationPanel
          progress={view.panelProgress}
          activeOperation={view.panelActiveOperation}
          operationError={view.panelError}
          canRetry={view.panelCanRetry}
          canCancel={view.panelCanCancel}
          isOperating={view.panelIsOperating}
          onCancel={onCancel}
          onRetry={onRetry}
          onClearError={onClearError}
        />
      )}
    </div>
  );
}

function BootstrapStatus() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  return (
    <div className="flex items-center gap-3 rounded-lg border bg-muted/40 p-3 text-sm text-muted-foreground">
      <Loader2 className="h-5 w-5 animate-spin" />
      <span>{t("actions.check-update")}</span>
    </div>
  );
}

function downloadLabel(label: string, version?: string) {
  return version ? `${label} ${version}` : label;
}
