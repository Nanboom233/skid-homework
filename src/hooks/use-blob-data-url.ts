"use client";

import {useEffect, useState} from "react";

export function useBlobDataUrl(blob: Blob | null | undefined): string | null {
  const [dataUrl, setDataUrl] = useState<{blob: Blob; value: string} | null>(null);

  useEffect(() => {
    if (!blob) {
      return;
    }

    let cancelled = false;
    const reader = new FileReader();

    reader.onload = () => {
      if (!cancelled && typeof reader.result === "string") {
        setDataUrl({blob, value: reader.result});
      }
    };
    reader.onerror = () => {
      if (!cancelled) {
        setDataUrl(null);
      }
    };
    reader.readAsDataURL(blob);

    return () => {
      cancelled = true;
      if (reader.readyState === FileReader.LOADING) {
        reader.abort();
      }
    };
  }, [blob]);

  if (!dataUrl || dataUrl.blob !== blob) {
    return null;
  }

  return dataUrl.value;
}
