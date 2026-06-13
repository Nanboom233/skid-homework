import {readFileSync} from "node:fs";

const frontendSource = readFileSync(
  "src-tauri/frontend/shared/scan-enhance.tsx",
  "utf8",
);
const rustSource = readFileSync("src-tauri/src/scan_enhance.rs", "utf8");

function extractStringConstant(source, name) {
  const match = source.match(
    new RegExp(`const\\s+${name}(?::\\s*&str)?\\s*=\\s*"([^"]+)"`),
  );
  if (!match) {
    throw new Error(`Unable to find ${name}.`);
  }
  return match[1];
}

const protocol = extractStringConstant(frontendSource, "ENHANCE_SCAN_PROTOCOL");
const host = extractStringConstant(frontendSource, "ENHANCE_SCAN_HOST");
const frontendPath = extractStringConstant(frontendSource, "ENHANCE_SCAN_PATH");
const rustPath = extractStringConstant(rustSource, "ENHANCE_SCAN_PATH");

if (frontendPath !== rustPath) {
  throw new Error(
    `Scan enhancement path drifted: frontend=${frontendPath} rust=${rustPath}`,
  );
}

const endpoint = `${protocol}://${host}${frontendPath}`;
if (endpoint !== "skidhw://localhost/enhance_scan") {
  throw new Error(`Unexpected scan enhancement endpoint: ${endpoint}`);
}

console.log(`Scan enhancement endpoint contract OK: ${endpoint}`);
