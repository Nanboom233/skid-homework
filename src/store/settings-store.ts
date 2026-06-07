import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

export type ThemePreference = "light" | "dark" | "system";
export type LanguagePreference = "en" | "zh";
export type ShortcutAction =
  | "upload"
  | "textInput"
  | "adbScreenshot"
  | "startScan"
  | "clearAll"
  | "openSettings"
  | "openChat"
  | "openGlobalTraitsEditor";

export type ShortcutMap = Record<ShortcutAction, string>;

export type ExplanationMode = "explanation" | "steps";
export type ScannerDetectionBackend = "opencv" | "native-ort";

const DEFAULT_SHORTCUTS: ShortcutMap = {
  upload: "ctrl+1",
  textInput: "ctrl+i",
  startScan: "ctrl+3",
  clearAll: "ctrl+4",
  openSettings: "ctrl+6",
  adbScreenshot: "ctrl+2",
  openChat: "ctrl+shift+o",
  openGlobalTraitsEditor: "ctrl+o",
};

const DEFAULT_LANGUAGE: LanguagePreference = "en";

export interface SettingsState {
  theme: ThemePreference;
  setThemePreference: (theme: ThemePreference) => void;

  language: LanguagePreference;
  languageInitialized: boolean;
  setLanguage: (language: LanguagePreference) => void;
  initializeLanguage: () => void;

  keybindings: ShortcutMap;
  setKeybinding: (action: ShortcutAction, binding: string) => void;
  resetKeybindings: () => void;

  traits: string;
  setTraits: (traits: string) => void;

  explanationMode: ExplanationMode;
  setExplanationMode: (explanationMode: ExplanationMode) => void;

  devtoolsEnabled: boolean;
  setDevtoolsState: (state: boolean) => void;

  clearDialogOnSubmit: boolean;
  setClearDialogOnSubmit: (state: boolean) => void;
  onlineSearchEnabled: boolean;
  setOnlineSearchEnabled: (state: boolean) => void;

  showModelSelectorInScanPage: boolean;
  setShowModelSelectorInScanPage: (state: boolean) => void;

  showOnlineSearchInScanPage: boolean;
  setShowOnlineSearchInScanPage: (state: boolean) => void;

  scannerDetectionBackend: ScannerDetectionBackend;
  setScannerDetectionBackend: (backend: ScannerDetectionBackend) => void;

  scannerNativeOrtStrictMode: boolean;
  setScannerNativeOrtStrictMode: (state: boolean) => void;

  scannerPipelineDebug: boolean;
  setScannerPipelineDebug: (state: boolean) => void;

  scannerPreviewWidth: number;
  scannerPreviewHeight: number;
  scannerFramerate: number;
  scannerCameraId: string;
  setScannerPreview: (
    width: number,
    height: number,
    framerate: number,
    cameraId: string,
  ) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      theme: "system",
      language: DEFAULT_LANGUAGE,
      languageInitialized: false,
      keybindings: { ...DEFAULT_SHORTCUTS },
      traits: "",
      explanationMode: "explanation",
      devtoolsEnabled: false,
      clearDialogOnSubmit: true,
      onlineSearchEnabled: false,
      showModelSelectorInScanPage: false,
      showOnlineSearchInScanPage: false,
      scannerDetectionBackend: "opencv",
      scannerNativeOrtStrictMode: false,
      scannerPipelineDebug: false,
      scannerPreviewWidth: 640,
      scannerPreviewHeight: 360,
      scannerFramerate: 30,
      scannerCameraId: "0",

      setThemePreference: (theme) => set({ theme }),
      setLanguage: (language) =>
        set({
          language,
          languageInitialized: true,
        }),
      initializeLanguage: () =>
        set((state) => {
          if (state.languageInitialized) {
            return state;
          }
          const prefersZh =
            typeof navigator !== "undefined" &&
            navigator.language.toLowerCase().startsWith("zh");
          return {
            languageInitialized: true,
            language: prefersZh ? "zh" : "en",
          };
        }),
      setKeybinding: (action, binding) =>
        set((state) => ({
          keybindings: {
            ...state.keybindings,
            [action]: binding,
          },
        })),
      resetKeybindings: () => set({ keybindings: { ...DEFAULT_SHORTCUTS } }),
      setTraits: (traits) => set({ traits }),
      setExplanationMode: (explanationMode) => set({ explanationMode }),
      setDevtoolsState: (state) => set({ devtoolsEnabled: state }),
      setClearDialogOnSubmit: (state) => set({ clearDialogOnSubmit: state }),
      setOnlineSearchEnabled: (state) => set({ onlineSearchEnabled: state }),
      setShowModelSelectorInScanPage: (state) =>
        set({ showModelSelectorInScanPage: state }),
      setShowOnlineSearchInScanPage: (state) =>
        set({ showOnlineSearchInScanPage: state }),
      setScannerDetectionBackend: (backend) =>
        set({ scannerDetectionBackend: backend }),
      setScannerNativeOrtStrictMode: (state) =>
        set({ scannerNativeOrtStrictMode: state }),
      setScannerPipelineDebug: (state) =>
        set({ scannerPipelineDebug: state }),
      setScannerPreview: (width, height, framerate, cameraId) =>
        set({
          scannerPreviewWidth: width,
          scannerPreviewHeight: height,
          scannerFramerate: framerate,
          scannerCameraId: cameraId,
        }),
    }),
    {
      name: "skidhw-storage",
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        theme: state.theme,
        language: state.language,
        languageInitialized: state.languageInitialized,
        keybindings: state.keybindings,
        traits: state.traits,
        explanationMode: state.explanationMode,
        devtoolsEnabled: state.devtoolsEnabled,
        clearDialogOnSubmit: state.clearDialogOnSubmit,
        onlineSearchEnabled: state.onlineSearchEnabled,
        showModelSelectorInScanPage: state.showModelSelectorInScanPage,
        showOnlineSearchInScanPage: state.showOnlineSearchInScanPage,
        scannerDetectionBackend: state.scannerDetectionBackend,
        scannerNativeOrtStrictMode: state.scannerNativeOrtStrictMode,
        scannerPipelineDebug: state.scannerPipelineDebug,
        scannerPreviewWidth: state.scannerPreviewWidth,
        scannerPreviewHeight: state.scannerPreviewHeight,
        scannerFramerate: state.scannerFramerate,
        scannerCameraId: state.scannerCameraId,
      }),
      version: 11,
      migrate: (persistedState, version) => {
        const data: Partial<SettingsState> & Record<string, unknown> =
          persistedState && typeof persistedState === "object"
            ? { ...(persistedState as Record<string, unknown>) }
            : {};

        if (version < 3) {
          data.keybindings = { ...DEFAULT_SHORTCUTS };
        }

        const existing = (data as { keybindings?: ShortcutMap }).keybindings;
        const legacyDevtools = (data as { devtools?: boolean }).devtools;
        const legacyShowModelSelectorInScanner = (
          data as { showModelSelectorInScanner?: boolean }
        ).showModelSelectorInScanner;
        const legacyShowOnlineSearchInScanner = (
          data as { showOnlineSearchInScanner?: boolean }
        ).showOnlineSearchInScanner;
        const rawDetectionBackend = (data as { scannerDetectionBackend?: unknown })
          .scannerDetectionBackend;
        const scannerDetectionBackend: ScannerDetectionBackend =
          rawDetectionBackend === "native-ort" || rawDetectionBackend === "opencv"
            ? rawDetectionBackend
            : "opencv";

        const migratedData = {
          ...data,
          keybindings: existing
            ? { ...DEFAULT_SHORTCUTS, ...existing }
            : { ...DEFAULT_SHORTCUTS },
          languageInitialized:
            (data as { languageInitialized?: boolean }).languageInitialized ??
            true,
          clearDialogOnSubmit:
            (data as { clearDialogOnSubmit?: boolean }).clearDialogOnSubmit ??
            true,
          onlineSearchEnabled:
            (data as { onlineSearchEnabled?: boolean }).onlineSearchEnabled ??
            false,
          showModelSelectorInScanPage:
            (data as { showModelSelectorInScanPage?: boolean })
              .showModelSelectorInScanPage ??
            legacyShowModelSelectorInScanner ??
            false,
          showOnlineSearchInScanPage:
            (data as { showOnlineSearchInScanPage?: boolean })
              .showOnlineSearchInScanPage ??
            legacyShowOnlineSearchInScanner ??
            false,
          devtoolsEnabled:
            (data as { devtoolsEnabled?: boolean }).devtoolsEnabled ??
            legacyDevtools ??
            false,
          scannerDetectionBackend,
          scannerNativeOrtStrictMode:
            (data as { scannerNativeOrtStrictMode?: boolean })
              .scannerNativeOrtStrictMode ?? false,
          scannerPipelineDebug:
            (data as { scannerPipelineDebug?: boolean }).scannerPipelineDebug ??
            false,
          scannerPreviewWidth:
            (data as { scannerPreviewWidth?: number }).scannerPreviewWidth ?? 640,
          scannerPreviewHeight:
            (data as { scannerPreviewHeight?: number }).scannerPreviewHeight ?? 360,
          scannerFramerate:
            (data as { scannerFramerate?: number }).scannerFramerate ?? 30,
          scannerCameraId:
            (data as { scannerCameraId?: string }).scannerCameraId ?? "0",
        };

        delete (migratedData as Record<string, unknown>).devtools;
        delete (migratedData as Record<string, unknown>).imageEnhancement;
        delete (migratedData as Record<string, unknown>).scannerPostProcessBackend;
        delete (migratedData as Record<string, unknown>)
          .showModelSelectorInScanner;
        delete (migratedData as Record<string, unknown>)
          .showOnlineSearchInScanner;
        return migratedData;
      },
    },
  ),
);

export const getDefaultShortcuts = () => ({ ...DEFAULT_SHORTCUTS });
