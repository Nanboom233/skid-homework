"use client";

import {
  type AiProvider,
  DEFAULT_GEMINI_BASE_URL,
  DEFAULT_OPENAI_BASE_URL,
  useAiStore,
} from "@/store/ai-store";
import {
  type LanguagePreference,
  type ScannerDetectionBackend,
  type ShortcutAction,
  type ThemePreference,
  useSettingsStore
} from "@/store/settings-store";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useHotkeys } from "react-hotkeys-hook";
import { useTranslation } from "react-i18next";
import { useMediaQuery } from "@/hooks/use-media-query";
import { useAvailableModels } from "@/hooks/use-available-models";
import {
  probeTauriScannerYolo,
  readTauriScannerYoloConfig,
  type TauriScannerYoloConfig,
  type TauriScannerYoloConfigResponse,
  type TauriScannerYoloLinuxConfig,
  type TauriScannerYoloModelConfig,
  type TauriScannerYoloProbeResult,
  type TauriScannerYoloWindowsConfig,
  writeTauriScannerYoloConfig,
} from "@/lib/tauri/scanner-detect";
import { isTauri } from "@/lib/tauri/platform";
import { toast } from "sonner";
import ShortcutRecorder from "./ShortcutRecorder";
import { useTheme } from "../theme-provider";
import { Button } from "../ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../ui/card";
import { Checkbox } from "../ui/checkbox";
import { Input } from "../ui/input";
import { Kbd } from "../ui/kbd";
import { Label } from "../ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { Slider } from "../ui/slider";
import { Textarea } from "../ui/textarea";
import AIAPICredentialsManager from "./AIAPICredentialsManager";
import AISourceManager from "./AISourceManager";
import ExplanationModeSelector from "./ExplanationModeSelector";
import ModelSelector, { CUSTOM_MODEL_VALUE } from "../ui/model-selector";
import { RefreshCw } from "lucide-react";

export const DEFAULT_BASE_BY_PROVIDER: Record<AiProvider, string> = {
  gemini: DEFAULT_GEMINI_BASE_URL,
  openai: DEFAULT_OPENAI_BASE_URL
};

const createDefaultScannerYoloWindowsConfig = (): TauriScannerYoloWindowsConfig => ({
  preferredProvider: "directml",
  runtimeLibrary: "onnxruntime/windows/onnxruntime.dll",
  sharedLibrary: "onnxruntime/windows/onnxruntime_providers_shared.dll",
  providerLibrary: "onnxruntime/windows/DirectML.dll",
});

const createDefaultScannerYoloLinuxConfig = (): TauriScannerYoloLinuxConfig => ({
  preferredProviders: ["tensorrt", "cuda"],
  runtimeLibrary: "onnxruntime/linux/libonnxruntime.so",
  providerLibraries: [
    "onnxruntime/linux/libonnxruntime_providers_tensorrt.so",
    "onnxruntime/linux/libonnxruntime_providers_cuda.so",
  ],
  officialGpuReleaseArtifact: "onnxruntime-linux-x64-gpu-1.24.4.tgz",
});

type BackButtonProps = {
  href?: string | null;
};

function BackButton({ href }: BackButtonProps) {
  const { t } = useTranslation("commons", {
    keyPrefix: "settings-page"
  });

  return (
    <div className="flex flex-col gap-3 sm:flex-row">
      <Link href={href ?? "/"} className="w-full sm:flex-1">
        <Button className="w-full">
          {t("back")} <Kbd>ESC</Kbd>
        </Button>
      </Link>
    </div>
  );
}

export default function SettingsPage() {
  const { t, i18n } = useTranslation("commons", {
    keyPrefix: "settings-page"
  });
  const isCompact = useMediaQuery("(max-width: 640px)");

  const searchParams = useSearchParams();

  const navTargetPath = searchParams.get("from");

  const {
    sources,
    activeSourceId,
    setActiveSource,
    fallbackModel,
    currentModel,
    updateSource,
    setFallbackModel,
    setCurrentModel,
    isCustomModel,
    setIsCustomModel,
    isCustomFallback,
    setIsCustomFallback,
    customModelName,
    setCustomModelName,
    customModelSourceId,
    setCustomModelSourceId,
    customFallbackName,
    setCustomFallbackName,
    customFallbackSourceId,
    setCustomFallbackSourceId
  } = useAiStore((s) => s);

  const {
    imageEnhancement: imageEnhancement,
    setImageEnhancement: setImageEnhancement,
    onlineSearchEnabled,
    setOnlineSearchEnabled,
    showModelSelectorInScanner,
    setShowModelSelectorInScanner,
    showOnlineSearchInScanner,
    setShowOnlineSearchInScanner,
    scannerDetectionBackend,
    setScannerDetectionBackend,
    scannerNativeYoloStrictMode,
    setScannerNativeYoloStrictMode,
    theme: themePreference,
    setThemePreference,
    language,
    setLanguage,
    keybindings,
    setKeybinding,
    resetKeybindings,
    devtoolsEnabled,
    setDevtoolsState,
    clearDialogOnSubmit,
    setClearDialogOnSubmit
  } = useSettingsStore((s) => s);

  const { theme: activeTheme, setTheme } = useTheme();
  const isDesktopTauri = isTauri();

  const [recordingAction, setRecordingAction] = useState<ShortcutAction | null>(
    null
  );
  const [scannerYoloConfig, setScannerYoloConfig] = useState<TauriScannerYoloConfig | null>(null);
  const [scannerYoloConfigMeta, setScannerYoloConfigMeta] =
    useState<Omit<TauriScannerYoloConfigResponse, "config"> | null>(null);
  const [scannerYoloProbe, setScannerYoloProbe] = useState<TauriScannerYoloProbeResult | null>(null);
  const [scannerYoloLoading, setScannerYoloLoading] = useState(false);
  const [scannerYoloSaving, setScannerYoloSaving] = useState(false);
  const [scannerYoloError, setScannerYoloError] = useState<string | null>(null);

  const activeSource = useMemo(
    () => sources.find((source) => source.id === activeSourceId) ?? sources[0],
    [sources, activeSourceId]
  );

  const localTraits = useMemo(() => activeSource?.traits ?? "", [activeSource]);
  const localThinkingBudget = useMemo(
    () => activeSource?.thinkingBudget ?? 8192,
    [activeSource]
  );

  // Get enabled sources for custom model provider selector
  const enabledSources = useMemo(
    () => sources.filter((source) => source.enabled && source.apiKey),
    [sources]
  );

  const router = useRouter();

  // Use shared hook for available models
  const {
    sourceModelsMap,
    isLoading: modelsLoading,
    forceRefetch
  } = useAvailableModels();

  const handleBack = useCallback(() => {
    if (navTargetPath) {
      router.push(navTargetPath);
    } else {
      router.push("/");
    }
  }, [router, navTargetPath]);
  useHotkeys("esc", handleBack);

  useEffect(() => {
    if (themePreference !== activeTheme) {
      setTheme(themePreference);
    }
  }, [themePreference, activeTheme, setTheme]);

  const themeOptions = useMemo(
    () => [
      {
        value: "system" as ThemePreference,
        label: t("appearance.theme.options.system")
      },
      {
        value: "light" as ThemePreference,
        label: t("appearance.theme.options.light")
      },
      {
        value: "dark" as ThemePreference,
        label: t("appearance.theme.options.dark")
      }
    ],
    [t]
  );

  const languageOptions = useMemo(
    () => [
      {
        value: "en" as LanguagePreference,
        label: t("appearance.language.options.en")
      },
      {
        value: "zh" as LanguagePreference,
        label: t("appearance.language.options.zh")
      }
    ],
    [t]
  );

  const handleThemeSelect = (value: ThemePreference) => {
    setThemePreference(value);
    setTheme(value);
  };

  const handleLanguageSelect = (value: LanguagePreference) => {
    setLanguage(value);
    if (i18n.language !== value) {
      i18n.changeLanguage(value);
    }
  };

  const [modelPopoverOpen, setModelPopoverOpen] = useState(false);
  const [fallbackPopoverOpen, setFallbackPopoverOpen] = useState(false);

  const translateSettings = useCallback(
    (key: string) => t(key as never) as string,
    [t]
  );

  const shortcutItems = useMemo(() => {
    return [
      {
        action: "upload" as ShortcutAction,
        label: translateSettings("shortcuts.actions.upload.label"),
        description: translateSettings("shortcuts.actions.upload.description")
      },
      {
        action: "textInput" as ShortcutAction,
        label: translateSettings("shortcuts.actions.text-input.label"),
        description: translateSettings(
          "shortcuts.actions.text-input.description"
        )
      },
      !isCompact && {
        action: "adbScreenshot" as ShortcutAction,
        label: translateSettings("shortcuts.actions.adb-screenshot.label"),
        description: translateSettings(
          "shortcuts.actions.adb-screenshot.description"
        )
      },
      {
        action: "startScan" as ShortcutAction,
        label: translateSettings("shortcuts.actions.start-scan.label"),
        description: translateSettings(
          "shortcuts.actions.start-scan.description"
        )
      },
      {
        action: "clearAll" as ShortcutAction,
        label: translateSettings("shortcuts.actions.clear-all.label"),
        description: translateSettings(
          "shortcuts.actions.clear-all.description"
        )
      },
      {
        action: "openSettings" as ShortcutAction,
        label: translateSettings("shortcuts.actions.open-settings.label"),
        description: translateSettings(
          "shortcuts.actions.open-settings.description"
        )
      },
      {
        action: "openChat" as ShortcutAction,
        label: translateSettings("shortcuts.actions.open-chat.label"),
        description: translateSettings(
          "shortcuts.actions.open-chat.description"
        )
      },
      {
        action: "openGlobalTraitsEditor" as ShortcutAction,
        label: translateSettings(
          "shortcuts.actions.open-global-traits-editor.label"
        ),
        description: translateSettings(
          "shortcuts.actions.open-global-traits-editor.description"
        )
      }
    ].filter(Boolean) as Array<{
      action: ShortcutAction;
      label: string;
      description: string;
    }>;
  }, [translateSettings, isCompact]);

  const shortcutsTitle = translateSettings("shortcuts.title");
  const shortcutsDesc = translateSettings("shortcuts.desc");
  const shortcutsResetLabel = translateSettings("shortcuts.reset");

  const handleModelChange = (model: string) => {
    if (model === CUSTOM_MODEL_VALUE) {
      setIsCustomModel(true);
    } else {
      setIsCustomModel(false);
      setCurrentModel(model);
    }
    setModelPopoverOpen(false);
  };

  const handleFallbackChange = (model: string, sourceId?: string | null) => {
    if (model === CUSTOM_MODEL_VALUE) {
      setIsCustomFallback(true);
    } else {
      setIsCustomFallback(false);
      setFallbackModel(model || null, sourceId ?? null);
    }
    setFallbackPopoverOpen(false);
  };

  const handleTraitsChange = (value: string) => {
    if (!activeSource) return;
    updateSource(activeSource.id, { traits: value || undefined });
  };

  const clearTraits = () => {
    if (!activeSource) return;
    updateSource(activeSource.id, { traits: undefined });
  };

  const handleThinkingBudgetChange = (value: number) => {
    if (!activeSource) return;
    updateSource(activeSource.id, { thinkingBudget: value });
  };

  const applyScannerYoloConfigResponse = useCallback(
    (response: TauriScannerYoloConfigResponse) => {
      setScannerYoloConfig(response.config);
      setScannerYoloConfigMeta({
        source: response.source,
        resolvedPath: response.resolvedPath,
        writablePath: response.writablePath,
      });
    },
    []
  );

  const loadScannerYoloDesktopState = useCallback(
    async (options?: { silent?: boolean }) => {
      if (!isDesktopTauri) {
        return;
      }

      const silent = options?.silent ?? false;
      if (!silent) {
        setScannerYoloLoading(true);
      }

      try {
        const [configResponse, probeResponse] = await Promise.all([
          readTauriScannerYoloConfig(),
          probeTauriScannerYolo(),
        ]);
        applyScannerYoloConfigResponse(configResponse);
        setScannerYoloProbe(probeResponse);
        setScannerYoloError(null);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setScannerYoloError(message);
        if (!silent) {
          toast.error(
            t("advanced.scanner-native-yolo-config.toasts.load-error", {
              error: message,
            })
          );
        }
      } finally {
        if (!silent) {
          setScannerYoloLoading(false);
        }
      }
    },
    [applyScannerYoloConfigResponse, isDesktopTauri, t]
  );

  useEffect(() => {
    if (!isDesktopTauri) {
      return;
    }

    void loadScannerYoloDesktopState({ silent: true });
  }, [isDesktopTauri, loadScannerYoloDesktopState]);

  const updateScannerYoloConfig = useCallback(
    (updater: (current: TauriScannerYoloConfig) => TauriScannerYoloConfig) => {
      setScannerYoloConfig((current) => (current ? updater(current) : current));
    },
    []
  );

  const updateScannerYoloModel = useCallback(
    (
      target: "intendedPrimaryModel" | "activePublicBaseline",
      patch: Partial<TauriScannerYoloModelConfig>
    ) => {
      updateScannerYoloConfig((current) => ({
        ...current,
        [target]: {
          ...current[target],
          ...patch,
        },
      }));
    },
    [updateScannerYoloConfig]
  );

  const updateScannerYoloWindows = useCallback(
    (patch: Partial<TauriScannerYoloWindowsConfig>) => {
      updateScannerYoloConfig((current) => ({
        ...current,
        windows: {
          ...(current.windows ?? createDefaultScannerYoloWindowsConfig()),
          ...patch,
        },
      }));
    },
    [updateScannerYoloConfig]
  );

  const updateScannerYoloLinux = useCallback(
    (patch: Partial<TauriScannerYoloLinuxConfig>) => {
      updateScannerYoloConfig((current) => ({
        ...current,
        linux: {
          ...(current.linux ?? createDefaultScannerYoloLinuxConfig()),
          ...patch,
        },
      }));
    },
    [updateScannerYoloConfig]
  );

  const handleBaselineInputSizeChange = (index: 0 | 1, rawValue: string) => {
    const parsed = Number.parseInt(rawValue, 10);
    updateScannerYoloConfig((current) => {
      const currentSize = current.activePublicBaseline.inputSize ?? [256, 256];
      const nextSize: [number, number] = [...currentSize] as [number, number];
      nextSize[index] = Number.isFinite(parsed) ? parsed : 0;

      return {
        ...current,
        activePublicBaseline: {
          ...current.activePublicBaseline,
          inputSize: nextSize,
        },
      };
    });
  };

  const handleScannerYoloSave = useCallback(async () => {
    if (!scannerYoloConfig) {
      return;
    }

    setScannerYoloSaving(true);
    try {
      const response = await writeTauriScannerYoloConfig(scannerYoloConfig);
      applyScannerYoloConfigResponse(response);
      setScannerYoloProbe(await probeTauriScannerYolo());
      setScannerYoloError(null);
      toast.success(t("advanced.scanner-native-yolo-config.toasts.save-success"));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setScannerYoloError(message);
      toast.error(
        t("advanced.scanner-native-yolo-config.toasts.save-error", {
          error: message,
        })
      );
    } finally {
      setScannerYoloSaving(false);
    }
  }, [applyScannerYoloConfigResponse, scannerYoloConfig, t]);

  const scannerYoloLinuxProviders = scannerYoloConfig?.linux?.preferredProviders.join(", ") ?? "";
  const scannerYoloNotes = scannerYoloConfig?.notes.join("\n") ?? "";

  return (
    <>
      <div className="mx-auto max-w-3xl space-y-8 p-4 md:p-8">
        <h1 className="text-2xl font-bold tracking-tight">{t("heading")}</h1>

        <BackButton href={navTargetPath} />

        <AISourceManager />

        <AIAPICredentialsManager
          key={activeSource.id}
          activeSource={activeSource}
        />

        <Card>
          <CardHeader>
            <CardTitle>{t("appearance.title")}</CardTitle>
            <CardDescription>{t("appearance.desc")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="theme-select">
                {t("appearance.theme.label")}
              </Label>
              <select
                id="theme-select"
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500"
                value={themePreference}
                onChange={(event) =>
                  handleThemeSelect(event.target.value as ThemePreference)
                }
              >
                {themeOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground">
                {t("appearance.theme.desc")}
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="language-select">
                {t("appearance.language.label")}
              </Label>
              <select
                id="language-select"
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500"
                value={language}
                onChange={(event) =>
                  handleLanguageSelect(event.target.value as LanguagePreference)
                }
              >
                {languageOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground">
                {t("appearance.language.desc")}
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{shortcutsTitle}</CardTitle>
            <CardDescription>{shortcutsDesc}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {shortcutItems.map((item) => (
              <div
                key={item.action}
                className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between"
              >
                <div>
                  <p className="font-medium">{item.label}</p>
                  <p className="text-sm text-muted-foreground">
                    {item.description}
                  </p>
                </div>
                <ShortcutRecorder
                  value={keybindings[item.action] ?? ""}
                  onChange={(combo) => setKeybinding(item.action, combo)}
                  isRecording={recordingAction === item.action}
                  onRecordingChange={(state) => {
                    if (!state) {
                      setRecordingAction(null);
                    } else {
                      setRecordingAction(item.action);
                    }
                  }}
                />
              </div>
            ))}
            <Button variant="ghost" onClick={resetKeybindings} className="mt-2">
              {shortcutsResetLabel}
            </Button>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("model.title")}</CardTitle>
            <CardDescription>{t("model.desc")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <ModelSelector
                sourceModelsMap={sourceModelsMap}
                value={currentModel}
                onChangeAction={handleModelChange}
                open={modelPopoverOpen}
                onOpenChangeAction={setModelPopoverOpen}
                allowCustom={true}
                isCustomSelected={isCustomModel}
                className="flex-2"
              />
              <Button
                variant="outline"
                size="icon"
                onClick={() => void forceRefetch()}
                disabled={modelsLoading}
                title={t("model.refresh")}
              >
                <RefreshCw
                  className={`h-4 w-4 ${modelsLoading ? "animate-spin" : ""}`}
                />
              </Button>
            </div>
            {isCustomModel && (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <Select
                    value={customModelSourceId}
                    onValueChange={(value) => {
                      setCustomModelSourceId(value);
                    }}
                  >
                    <SelectTrigger className="w-45">
                      <SelectValue
                        placeholder={t("model.manual.select-provider")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {enabledSources.map((source) => (
                        <SelectItem key={source.id} value={source.id}>
                          {source.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Input
                    id="model-manual"
                    className="flex-1"
                    value={customModelName}
                    onChange={(event) => {
                      setCustomModelName(event.target.value);
                    }}
                    onBlur={() => {
                      // Only apply custom model when user finishes editing
                      if (customModelSourceId && customModelName.trim()) {
                        setActiveSource(customModelSourceId);
                        setCurrentModel(customModelName.trim());
                      }
                    }}
                    placeholder={t("model.manual.placeholder")}
                  />
                </div>
                <p className="text-xs text-muted-foreground">
                  {t("model.manual.desc")}
                </p>
              </div>
            )}
            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <Checkbox
                  id="show-model-selector"
                  checked={showModelSelectorInScanner}
                  onCheckedChange={(state) =>
                    setShowModelSelectorInScanner(Boolean(state))
                  }
                />
                <Label htmlFor="show-model-selector">
                  {t("model.show-selector-in-scanner")}
                </Label>
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="max-retries">
                {t("model.max-retries.label")}
              </Label>
              <div className="flex items-center gap-2">
                <div className="flex-1">
                  <Slider
                    value={[activeSource?.maxRetries ?? 5]}
                    onValueChange={(value) => {
                      if (!activeSource) return;
                      updateSource(activeSource.id, { maxRetries: value[0] });
                    }}
                    min={0}
                    max={10}
                    step={1}
                  />
                </div>
                <Input
                  id="max-retries"
                  className="w-16"
                  type="number"
                  min={0}
                  max={10}
                  value={activeSource?.maxRetries ?? 5}
                  onChange={(event) => {
                    if (!activeSource) return;
                    const val = parseInt(event.target.value, 10);
                    updateSource(activeSource.id, {
                      maxRetries: isNaN(val)
                        ? 5
                        : Math.max(0, Math.min(10, val))
                    });
                  }}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("model.max-retries.tip")}
              </p>
            </div>
            <div className="space-y-2">
              <Label>{t("model.fallback.label")}</Label>
              <ModelSelector
                sourceModelsMap={sourceModelsMap}
                value={fallbackModel}
                onChangeAction={handleFallbackChange}
                open={fallbackPopoverOpen}
                onOpenChangeAction={setFallbackPopoverOpen}
                allowNone={true}
                noneLabel={t("model.fallback.none")}
                allowCustom={true}
                isCustomSelected={isCustomFallback}
                excludeModel={currentModel}
                className="w-full"
              />
              {isCustomFallback && (
                <div className="flex items-center gap-2 mt-2">
                  <Select
                    value={customFallbackSourceId}
                    onValueChange={(value) => {
                      setCustomFallbackSourceId(value);
                    }}
                  >
                    <SelectTrigger className="w-45">
                      <SelectValue
                        placeholder={t("model.manual.select-provider")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {enabledSources.map((source) => (
                        <SelectItem key={source.id} value={source.id}>
                          {source.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Input
                    className="flex-1"
                    value={customFallbackName}
                    onChange={(event) => {
                      setCustomFallbackName(event.target.value);
                    }}
                    onBlur={() => {
                      // Only apply custom fallback model when user finishes editing
                      if (customFallbackSourceId && customFallbackName.trim()) {
                        setFallbackModel(customFallbackName.trim());
                      }
                    }}
                    placeholder={t("model.manual.placeholder")}
                  />
                </div>
              )}
              <p className="text-xs text-muted-foreground">
                {t("model.fallback.desc")}
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("thinking.title")}</CardTitle>
            <CardDescription>{t("thinking.desc")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {activeSource?.provider === "gemini" && (
              <div className="space-y-2">
                <Label>{t("thinking.budget")}</Label>
                <div className="flex items-center gap-2">
                  <div className="flex-1">
                    <Slider
                      value={[localThinkingBudget]}
                      onValueChange={(value) =>
                        handleThinkingBudgetChange(value[0])
                      }
                      min={128}
                      max={24576}
                      step={1}
                    />
                  </div>
                  <Input
                    className="w-24"
                    value={localThinkingBudget}
                    type="number"
                    min={128}
                    max={24576}
                    onChange={(event) =>
                      handleThinkingBudgetChange(
                        Math.max(
                          128,
                          Math.min(24576, Number(event.target.value) || 128)
                        )
                      )
                    }
                  />
                  <span>{t("thinking.tokens-unit")}</span>
                </div>
              </div>
            )}

            <div className="space-y-2">
              <Label htmlFor="online-search-toggle">
                {t("thinking.online-search.title")}
              </Label>
              <div className="flex items-center gap-3">
                <Checkbox
                  id="online-search-toggle"
                  checked={onlineSearchEnabled}
                  onCheckedChange={(state) =>
                    setOnlineSearchEnabled(state === true)
                  }
                />
                <div className="space-y-1">
                  <p className="text-sm font-medium">
                    {t("thinking.online-search.toggle.settings")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("thinking.online-search.desc")}
                  </p>
                </div>
              </div>
            </div>

            <div className="space-y-2">
              <div className="flex items-center gap-3">
                <Checkbox
                  id="show-online-search-scanner"
                  checked={showOnlineSearchInScanner}
                  onCheckedChange={(state) =>
                    setShowOnlineSearchInScanner(state === true)
                  }
                />
                <Label htmlFor="show-online-search-scanner">
                  {t("thinking.online-search.show-toggle-in-scanner")}
                </Label>
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="traits-input">{t("traits.title")}</Label>
              <div className="relative">
                <Textarea
                  id="traits-input"
                  className="min-h-25 pr-20"
                  value={localTraits}
                  onChange={(event) => handleTraitsChange(event.target.value)}
                  placeholder={t("traits.placeholder")}
                />
                <Button
                  variant="ghost"
                  size="sm"
                  className="absolute right-2 top-2"
                  onClick={clearTraits}
                  disabled={!localTraits}
                >
                  {t("clear-input")}
                </Button>
              </div>
              <p className="text-sm text-muted-foreground">
                {t("traits.desc")}
              </p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t("advanced.title")}</CardTitle>
            <CardDescription>{t("advanced.desc")}</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <div className="flex items-center gap-3">
              <Checkbox
                id="image-enhancement"
                checked={imageEnhancement}
                onCheckedChange={(state) =>
                  setImageEnhancement(state as boolean)
                }
              />
              <Label htmlFor="image-enhancement">
                {t("advanced.image-post-processing.enhancement")}
              </Label>
            </div>

            <div className="space-y-2">
              <Label htmlFor="scanner-detection-backend">
                {t("advanced.scanner-detection-backend.label")}
              </Label>
              <Select
                value={scannerDetectionBackend}
                onValueChange={(value) =>
                  setScannerDetectionBackend(value as ScannerDetectionBackend)
                }
              >
                <SelectTrigger id="scanner-detection-backend">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="opencv">
                    {t("advanced.scanner-detection-backend.options.opencv")}
                  </SelectItem>
                  <SelectItem value="native-yolo">
                    {t("advanced.scanner-detection-backend.options.native-yolo")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <p className="text-sm text-muted-foreground">
                {t("advanced.scanner-detection-backend.desc")}
              </p>
            </div>

            <div className="flex items-center gap-3">
              <Checkbox
                id="scanner-native-yolo-strict-mode"
                checked={scannerNativeYoloStrictMode}
                onCheckedChange={(state) =>
                  setScannerNativeYoloStrictMode(state === true)
                }
              />
              <div className="space-y-1">
                <Label htmlFor="scanner-native-yolo-strict-mode">
                  {t("advanced.scanner-native-yolo-strict-mode.label")}
                </Label>
                <p className="text-sm text-muted-foreground">
                  {t("advanced.scanner-native-yolo-strict-mode.desc")}
                </p>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <Checkbox
                id="devtools-enabled"
                checked={devtoolsEnabled}
                onCheckedChange={(state) => setDevtoolsState(Boolean(state))}
              />
              <Label htmlFor="devtools-enabled">Enable Devtools</Label>
            </div>

            <div className="flex items-center gap-3">
              <Checkbox
                id="clear-dialog-on-submit"
                checked={clearDialogOnSubmit}
                onCheckedChange={(state) =>
                  setClearDialogOnSubmit(state === true)
                }
              />
              <Label htmlFor="clear-dialog-on-submit">
                {t("advanced.ui.clear-dialog-on-submit")}
              </Label>
            </div>

            <div className="flex items-center gap-3">
              <Label>{t("advanced.explanation.title")}</Label>

              <ExplanationModeSelector />
            </div>
          </CardContent>
        </Card>

        {isDesktopTauri ? (
          <Card>
            <CardHeader>
              <CardTitle>{t("advanced.scanner-native-yolo-config.title")}</CardTitle>
              <CardDescription>{t("advanced.scanner-native-yolo-config.desc")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="flex flex-wrap gap-3">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void loadScannerYoloDesktopState()}
                  disabled={scannerYoloLoading || scannerYoloSaving}
                >
                  <RefreshCw
                    className={`mr-2 h-4 w-4 ${(scannerYoloLoading || scannerYoloSaving) ? "animate-spin" : ""}`}
                  />
                  {t("advanced.scanner-native-yolo-config.actions.reload")}
                </Button>
                <Button
                  type="button"
                  onClick={() => void handleScannerYoloSave()}
                  disabled={!scannerYoloConfig || scannerYoloLoading || scannerYoloSaving}
                >
                  {scannerYoloSaving
                    ? t("advanced.scanner-native-yolo-config.actions.saving")
                    : t("advanced.scanner-native-yolo-config.actions.save")}
                </Button>
              </div>

              {scannerYoloError ? (
                <p className="text-sm text-destructive">{scannerYoloError}</p>
              ) : null}

              <div className="grid gap-3 md:grid-cols-2">
                <div className="space-y-2">
                  <Label>{t("advanced.scanner-native-yolo-config.status.config-source")}</Label>
                  <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-xs font-mono">
                    {scannerYoloConfigMeta?.source ?? scannerYoloProbe?.configSource ?? "—"}
                  </p>
                </div>
                <div className="space-y-2">
                  <Label>{t("advanced.scanner-native-yolo-config.status.resolved-path")}</Label>
                  <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-xs font-mono">
                    {scannerYoloConfigMeta?.resolvedPath ?? scannerYoloProbe?.configPath ?? "—"}
                  </p>
                </div>
                <div className="space-y-2 md:col-span-2">
                  <Label>{t("advanced.scanner-native-yolo-config.status.writable-path")}</Label>
                  <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-xs font-mono">
                    {scannerYoloConfigMeta?.writablePath ?? "—"}
                  </p>
                </div>
              </div>

              {scannerYoloProbe ? (
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label>{t("advanced.scanner-native-yolo-config.status.runtime-ready")}</Label>
                    <p className="rounded-md border bg-muted/40 px-3 py-2 text-sm">
                      {scannerYoloProbe.runtimeReady
                        ? t("advanced.scanner-native-yolo-config.state.ready")
                        : t("advanced.scanner-native-yolo-config.state.not-ready")}
                    </p>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("advanced.scanner-native-yolo-config.status.session-ready")}</Label>
                    <p className="rounded-md border bg-muted/40 px-3 py-2 text-sm">
                      {scannerYoloProbe.sessionReady
                        ? t("advanced.scanner-native-yolo-config.state.ready")
                        : t("advanced.scanner-native-yolo-config.state.not-ready")}
                    </p>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("advanced.scanner-native-yolo-config.status.provider")}</Label>
                    <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-sm">
                      {scannerYoloProbe.preferredProvider || "—"}
                    </p>
                  </div>
                  <div className="space-y-2">
                    <Label>{t("advanced.scanner-native-yolo-config.status.model")}</Label>
                    <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-sm">
                      {scannerYoloProbe.selectedModelId || "—"}
                    </p>
                  </div>
                  <div className="space-y-2 md:col-span-2">
                    <Label>{t("advanced.scanner-native-yolo-config.status.message")}</Label>
                    <p className="break-all rounded-md border bg-muted/40 px-3 py-2 text-sm">
                      {scannerYoloProbe.message}
                    </p>
                  </div>
                </div>
              ) : null}

              {scannerYoloConfig ? (
                <>
                  <div className="grid gap-3 md:grid-cols-2">
                    <div className="space-y-2">
                      <Label htmlFor="scanner-yolo-stage">
                        {t("advanced.scanner-native-yolo-config.fields.stage")}
                      </Label>
                      <Input
                        id="scanner-yolo-stage"
                        value={scannerYoloConfig.stage}
                        onChange={(event) =>
                          updateScannerYoloConfig((current) => ({
                            ...current,
                            stage: event.target.value,
                          }))
                        }
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="scanner-yolo-task">
                        {t("advanced.scanner-native-yolo-config.fields.task")}
                      </Label>
                      <Input
                        id="scanner-yolo-task"
                        value={scannerYoloConfig.task}
                        onChange={(event) =>
                          updateScannerYoloConfig((current) => ({
                            ...current,
                            task: event.target.value,
                          }))
                        }
                      />
                    </div>
                  </div>

                  <div className="grid gap-6 lg:grid-cols-2">
                    <div className="space-y-3 rounded-lg border p-4">
                      <h3 className="text-sm font-semibold">
                        {t("advanced.scanner-native-yolo-config.fields.intended-primary.title")}
                      </h3>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-intended-id">
                          {t("advanced.scanner-native-yolo-config.fields.id")}
                        </Label>
                        <Input
                          id="scanner-yolo-intended-id"
                          value={scannerYoloConfig.intendedPrimaryModel.id}
                          onChange={(event) =>
                            updateScannerYoloModel("intendedPrimaryModel", {
                              id: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-intended-kind">
                          {t("advanced.scanner-native-yolo-config.fields.kind")}
                        </Label>
                        <Input
                          id="scanner-yolo-intended-kind"
                          value={scannerYoloConfig.intendedPrimaryModel.kind}
                          onChange={(event) =>
                            updateScannerYoloModel("intendedPrimaryModel", {
                              kind: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-intended-task">
                          {t("advanced.scanner-native-yolo-config.fields.model-task")}
                        </Label>
                        <Input
                          id="scanner-yolo-intended-task"
                          value={scannerYoloConfig.intendedPrimaryModel.task}
                          onChange={(event) =>
                            updateScannerYoloModel("intendedPrimaryModel", {
                              task: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-intended-path">
                          {t("advanced.scanner-native-yolo-config.fields.model-path")}
                        </Label>
                        <Input
                          id="scanner-yolo-intended-path"
                          value={scannerYoloConfig.intendedPrimaryModel.modelPath}
                          onChange={(event) =>
                            updateScannerYoloModel("intendedPrimaryModel", {
                              modelPath: event.target.value,
                            })
                          }
                        />
                      </div>
                    </div>

                    <div className="space-y-3 rounded-lg border p-4">
                      <h3 className="text-sm font-semibold">
                        {t("advanced.scanner-native-yolo-config.fields.active-public-baseline.title")}
                      </h3>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-baseline-id">
                          {t("advanced.scanner-native-yolo-config.fields.id")}
                        </Label>
                        <Input
                          id="scanner-yolo-baseline-id"
                          value={scannerYoloConfig.activePublicBaseline.id}
                          onChange={(event) =>
                            updateScannerYoloModel("activePublicBaseline", {
                              id: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-baseline-kind">
                          {t("advanced.scanner-native-yolo-config.fields.kind")}
                        </Label>
                        <Input
                          id="scanner-yolo-baseline-kind"
                          value={scannerYoloConfig.activePublicBaseline.kind}
                          onChange={(event) =>
                            updateScannerYoloModel("activePublicBaseline", {
                              kind: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-baseline-task">
                          {t("advanced.scanner-native-yolo-config.fields.model-task")}
                        </Label>
                        <Input
                          id="scanner-yolo-baseline-task"
                          value={scannerYoloConfig.activePublicBaseline.task}
                          onChange={(event) =>
                            updateScannerYoloModel("activePublicBaseline", {
                              task: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="scanner-yolo-baseline-path">
                          {t("advanced.scanner-native-yolo-config.fields.model-path")}
                        </Label>
                        <Input
                          id="scanner-yolo-baseline-path"
                          value={scannerYoloConfig.activePublicBaseline.modelPath}
                          onChange={(event) =>
                            updateScannerYoloModel("activePublicBaseline", {
                              modelPath: event.target.value,
                            })
                          }
                        />
                      </div>
                      <div className="grid gap-3 md:grid-cols-2">
                        <div className="space-y-2">
                          <Label htmlFor="scanner-yolo-baseline-input-name">
                            {t("advanced.scanner-native-yolo-config.fields.input-name")}
                          </Label>
                          <Input
                            id="scanner-yolo-baseline-input-name"
                            value={scannerYoloConfig.activePublicBaseline.inputName ?? ""}
                            onChange={(event) =>
                              updateScannerYoloModel("activePublicBaseline", {
                                inputName: event.target.value || null,
                              })
                            }
                          />
                        </div>
                        <div className="space-y-2">
                          <Label htmlFor="scanner-yolo-baseline-output-name">
                            {t("advanced.scanner-native-yolo-config.fields.output-name")}
                          </Label>
                          <Input
                            id="scanner-yolo-baseline-output-name"
                            value={scannerYoloConfig.activePublicBaseline.outputName ?? ""}
                            onChange={(event) =>
                              updateScannerYoloModel("activePublicBaseline", {
                                outputName: event.target.value || null,
                              })
                            }
                          />
                        </div>
                      </div>
                      <div className="grid gap-3 md:grid-cols-2">
                        <div className="space-y-2">
                          <Label htmlFor="scanner-yolo-baseline-input-width">
                            {t("advanced.scanner-native-yolo-config.fields.input-width")}
                          </Label>
                          <Input
                            id="scanner-yolo-baseline-input-width"
                            type="number"
                            inputMode="numeric"
                            value={scannerYoloConfig.activePublicBaseline.inputSize?.[0] ?? 0}
                            onChange={(event) =>
                              handleBaselineInputSizeChange(0, event.target.value)
                            }
                          />
                        </div>
                        <div className="space-y-2">
                          <Label htmlFor="scanner-yolo-baseline-input-height">
                            {t("advanced.scanner-native-yolo-config.fields.input-height")}
                          </Label>
                          <Input
                            id="scanner-yolo-baseline-input-height"
                            type="number"
                            inputMode="numeric"
                            value={scannerYoloConfig.activePublicBaseline.inputSize?.[1] ?? 0}
                            onChange={(event) =>
                              handleBaselineInputSizeChange(1, event.target.value)
                            }
                          />
                        </div>
                      </div>
                    </div>
                  </div>

                  <div className="grid gap-3 md:grid-cols-2">
                    <div className="space-y-2">
                      <Label htmlFor="scanner-yolo-windows-provider">
                        {t("advanced.scanner-native-yolo-config.fields.windows-provider")}
                      </Label>
                      <Input
                        id="scanner-yolo-windows-provider"
                        value={scannerYoloConfig.windows?.preferredProvider ?? ""}
                        onChange={(event) =>
                          updateScannerYoloWindows({
                            preferredProvider: event.target.value,
                          })
                        }
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="scanner-yolo-linux-providers">
                        {t("advanced.scanner-native-yolo-config.fields.linux-providers")}
                      </Label>
                      <Input
                        id="scanner-yolo-linux-providers"
                        value={scannerYoloLinuxProviders}
                        onChange={(event) =>
                          updateScannerYoloLinux({
                            preferredProviders: event.target.value
                              .split(",")
                              .map((provider) => provider.trim())
                              .filter(Boolean),
                          })
                        }
                      />
                    </div>
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="scanner-yolo-notes">
                      {t("advanced.scanner-native-yolo-config.fields.notes")}
                    </Label>
                    <Textarea
                      id="scanner-yolo-notes"
                      className="min-h-28"
                      value={scannerYoloNotes}
                      onChange={(event) =>
                        updateScannerYoloConfig((current) => ({
                          ...current,
                          notes: event.target.value
                            .split("\n")
                            .map((line) => line.trim())
                            .filter(Boolean),
                        }))
                      }
                    />
                  </div>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">
                  {t("advanced.scanner-native-yolo-config.empty")}
                </p>
              )}
            </CardContent>
          </Card>
        ) : null}

        <BackButton href={navTargetPath} />
      </div>
    </>
  );
}
