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
export type ScannerPostProcessBackend = "heuristic" | "native-ml-v1";

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
  imageEnhancement: boolean;
  setImageEnhancement: (imagePostprocessing: boolean) => void;

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

  showModelSelectorInScanner: boolean;
  setShowModelSelectorInScanner: (state: boolean) => void;

  showOnlineSearchInScanner: boolean;
  setShowOnlineSearchInScanner: (state: boolean) => void;

  scannerDetectionBackend: ScannerDetectionBackend;
  setScannerDetectionBackend: (backend: ScannerDetectionBackend) => void;

  scannerNativeOrtStrictMode: boolean;
  setScannerNativeOrtStrictMode: (state: boolean) => void;

  scannerPostProcessBackend: ScannerPostProcessBackend;
  setScannerPostProcessBackend: (backend: ScannerPostProcessBackend) => void;

  scannerPipelineDebug: boolean;
  setScannerPipelineDebug: (state: boolean) => void;

  scannerPreviewWidth: number;
  scannerPreviewHeight: number;
  scannerFramerate: number;
  scannerCameraId: string;
  setScannerPreview: (width: number, height: number, framerate: number, cameraId: string) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      imageEnhancement: true,
      theme: "system",
      language: DEFAULT_LANGUAGE,
      languageInitialized: false,
      keybindings: { ...DEFAULT_SHORTCUTS },
      traits: "",
      explanationMode: "explanation",
      devtoolsEnabled: false,
      clearDialogOnSubmit: true,
      onlineSearchEnabled: false,
      showModelSelectorInScanner: false,
      showOnlineSearchInScanner: false,
      scannerDetectionBackend: "opencv",
      scannerNativeOrtStrictMode: false,
      scannerPostProcessBackend: "heuristic",
      scannerPipelineDebug: false,
      scannerPreviewWidth: 640,
      scannerPreviewHeight: 360,
      scannerFramerate: 30,
      scannerCameraId: "0",

      setImageEnhancement: (state) => set({ imageEnhancement: state }),
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
      setShowModelSelectorInScanner: (state) =>
        set({ showModelSelectorInScanner: state }),
      setShowOnlineSearchInScanner: (state) =>
        set({ showOnlineSearchInScanner: state }),
      setScannerDetectionBackend: (backend) =>
        set({ scannerDetectionBackend: backend }),
      setScannerNativeOrtStrictMode: (state) =>
        set({ scannerNativeOrtStrictMode: state }),
      setScannerPostProcessBackend: (backend) =>
        set({ scannerPostProcessBackend: backend }),
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
        imageEnhancement: state.imageEnhancement,
        theme: state.theme,
        language: state.language,
        languageInitialized: state.languageInitialized,
        keybindings: state.keybindings,
        traits: state.traits,
        explanationMode: state.explanationMode,
        devtoolsEnabled: state.devtoolsEnabled,
        clearDialogOnSubmit: state.clearDialogOnSubmit,
        onlineSearchEnabled: state.onlineSearchEnabled,
        showModelSelectorInScanner: state.showModelSelectorInScanner,
        showOnlineSearchInScanner: state.showOnlineSearchInScanner,
        scannerDetectionBackend: state.scannerDetectionBackend,
        scannerNativeOrtStrictMode: state.scannerNativeOrtStrictMode,
        scannerPostProcessBackend: state.scannerPostProcessBackend,
        scannerPipelineDebug: state.scannerPipelineDebug,
        scannerPreviewWidth: state.scannerPreviewWidth,
        scannerPreviewHeight: state.scannerPreviewHeight,
        scannerFramerate: state.scannerFramerate,
        scannerCameraId: state.scannerCameraId,
      }),
      version: 13,
      migrate: (persistedState) => {
        const raw: Record<string, unknown> =
          persistedState && typeof persistedState === "object"
            ? { ...(persistedState as Record<string, unknown>) }
            : {};

        // --- Keybindings: merge persisted shortcuts over defaults ---
        const existingKeybindings = raw.keybindings as ShortcutMap | undefined;
        const keybindings = existingKeybindings
          ? { ...DEFAULT_SHORTCUTS, ...existingKeybindings }
          : { ...DEFAULT_SHORTCUTS };

        // --- Enum validation helpers ---
        const VALID_DETECTION_BACKENDS: readonly string[] = ["opencv", "native-ort"];
        const VALID_POSTPROCESS_BACKENDS: readonly string[] = ["heuristic", "native-ml-v1"];

        const validatedDetectionBackend = VALID_DETECTION_BACKENDS.includes(raw.scannerDetectionBackend as string)
          ? (raw.scannerDetectionBackend as ScannerDetectionBackend)
          : "native-ort";
        const validatedPostProcessBackend = VALID_POSTPROCESS_BACKENDS.includes(raw.scannerPostProcessBackend as string)
          ? (raw.scannerPostProcessBackend as ScannerPostProcessBackend)
          : "heuristic";

        // --- Build fully-resolved state (every field gets a valid value) ---
        const migrated = {
          ...raw,
          keybindings,
          languageInitialized: (raw.languageInitialized as boolean | undefined) ?? true,
          clearDialogOnSubmit: (raw.clearDialogOnSubmit as boolean | undefined) ?? true,
          onlineSearchEnabled: (raw.onlineSearchEnabled as boolean | undefined) ?? false,
          showModelSelectorInScanner: (raw.showModelSelectorInScanner as boolean | undefined) ?? false,
          showOnlineSearchInScanner: (raw.showOnlineSearchInScanner as boolean | undefined) ?? false,
          scannerDetectionBackend: validatedDetectionBackend,
          scannerNativeOrtStrictMode: (raw.scannerNativeOrtStrictMode as boolean | undefined) ?? false,
          scannerPostProcessBackend: validatedPostProcessBackend,
          scannerPipelineDebug: (raw.scannerPipelineDebug as boolean | undefined) ?? false,
          scannerPreviewWidth: (raw.scannerPreviewWidth as number | undefined) ?? 640,
          scannerPreviewHeight: (raw.scannerPreviewHeight as number | undefined) ?? 360,
          scannerFramerate: (raw.scannerFramerate as number | undefined) ?? 30,
          scannerCameraId: (raw.scannerCameraId as string | undefined) ?? "0",
          devtoolsEnabled:
            (raw.devtoolsEnabled as boolean | undefined) ??
            (raw.devtools as boolean | undefined) ??
            false,
        };

        // --- Remove legacy keys ---
        delete (migrated as Record<string, unknown>).devtools;

        return migrated;
      },
    },
  ),
);

export const getDefaultShortcuts = () => ({ ...DEFAULT_SHORTCUTS });
