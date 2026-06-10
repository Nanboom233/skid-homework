"use client";

import {useCallback} from "react";
import {useTranslation} from "react-i18next";
import type {AssetTarget} from "../../lib/tauri/scanner";
import {useScannerStore} from "../../store/scanner-store";

export function useScannerArchiveImport(target: AssetTarget) {
  const {t} = useTranslation("commons", {keyPrefix: "scanner"});

  return useCallback(async () => {
    const {open} = await import("@tauri-apps/plugin-dialog");
    const isWindows = navigator.userAgent.includes("Windows");
    const extensions =
      target === "camera-server"
        ? ["jar"]
        : isWindows
          ? ["zip"]
          : ["tar.gz"];
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: t("import.filter-label"),
          extensions,
        },
      ],
    });
    if (selected) {
      const {startImport} = useScannerStore.getState();
      await startImport(target, selected);
    }
  }, [t, target]);
}
