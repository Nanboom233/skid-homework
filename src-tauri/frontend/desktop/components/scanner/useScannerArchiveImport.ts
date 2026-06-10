"use client";

import {useCallback} from "react";
import {useTranslation} from "react-i18next";
import {useScannerStore} from "../../store/scanner-store";

export function useScannerArchiveImport() {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});

  return useCallback(async () => {
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
  }, [t]);
}
