"use client";

import {useEffect} from "react";
import {useTranslation} from "react-i18next";
import {
  CheckCircle2,
  XCircle,
  Download,
  FolderOpen,
  AlertTriangle,
  Loader2,
  Cpu,
  Zap,
} from "lucide-react";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from "@/components/ui/card";
import {Collapsible, CollapsibleContent, CollapsibleTrigger} from "@/components/ui/collapsible";
import {Badge} from "@/components/ui/badge";
import {useScannerStore} from "../../store/scanner-store";

export default function ScannerSettingsCard() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});
  const assetsStatus = useScannerStore((s) => s.assetsStatus);
  const probeStatus = useScannerStore((s) => s.probeStatus);
  const progress = useScannerStore((s) => s.progress);
  const isOperating = useScannerStore((s) => s.isOperating);
  const operationError = useScannerStore((s) => s.operationError);
  const fetchStatus = useScannerStore((s) => s.fetchStatus);
  const fetchProbe = useScannerStore((s) => s.fetchProbe);
  const startDownload = useScannerStore((s) => s.startDownload);
  const clearError = useScannerStore((s) => s.clearError);

  useEffect(() => {
    void fetchStatus();
    void fetchProbe();
  }, [fetchStatus, fetchProbe]);

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
            {assetsStatus?.platformTarget && (
              <Badge variant="outline">{assetsStatus.platformTarget}</Badge>
            )}
            {probeStatus && (
              <Badge variant={probeStatus.preferredProviderReady ? "default" : "secondary"}>
                {probeStatus.preferredProviderReady ? (
                  <><Zap className="mr-1 h-3 w-3" />{probeStatus.preferredProvider}</>
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
            onClick={() => void startDownload()}
            disabled={isOperating}
          >
            <Download className="mr-2 h-4 w-4" />
            {isReady ? t("actions.update") : t("actions.download")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void handleImport()}
            disabled={isOperating}
          >
            <FolderOpen className="mr-2 h-4 w-4" />
            {t("actions.import")}
          </Button>
        </div>

        {isOperating && progress && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span className="text-sm">{t(`phases.${progress.phase}`)}</span>
            </div>
            {progress.bytesTotal != null && progress.bytesTotal > 0 && (
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
          <div className="space-y-2 rounded-lg border border-red-200 bg-red-50 p-3 dark:border-red-800 dark:bg-red-950">
            <div className="flex items-start gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 text-red-600 dark:text-red-400" />
              <div className="flex-1">
                <p className="text-sm font-medium text-red-800 dark:text-red-200">
                  {t("error.title")}
                </p>
                <p className="mt-1 text-xs text-red-600 dark:text-red-400">
                  {operationError.details ?? operationError.code}
                </p>
              </div>
            </div>
            <div className="flex gap-2">
              {operationError.retryable && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    clearError();
                    void startDownload();
                  }}
                >
                  {t("actions.retry")}
                </Button>
              )}
              <Button size="sm" variant="outline" onClick={() => void handleImport()}>
                <FolderOpen className="mr-1 h-3 w-3" />
                {t("actions.import")}
              </Button>
            </div>
          </div>
        )}

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

  return (
    <div className="space-y-2 text-xs">
      <DiagRow label={t("runtime-path")} value={probeStatus.runtimeLibraryPath} />
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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
