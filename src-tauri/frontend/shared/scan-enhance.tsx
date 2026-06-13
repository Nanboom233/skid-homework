const ENHANCE_SCAN_ENDPOINT = "skidhw://localhost/enhance_scan";

export function ImagePostProcessLoader(): React.JSX.Element | null {
  return null;
}

export async function processImage(
  file: File,
): Promise<{file: File; url: string}> {
  const response = await fetch(ENHANCE_SCAN_ENDPOINT, {
    method: "POST",
    body: file,
  });

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    const message = detail.trim() || `Image enhancement failed (${response.status}).`;
    throw new Error(message);
  }

  const bytes = await response.arrayBuffer();
  const enhancedFile = new File([bytes], `enhanced_${file.name}.png`, {
    type: "image/png",
  });

  return {
    file: enhancedFile,
    url: URL.createObjectURL(enhancedFile),
  };
}
