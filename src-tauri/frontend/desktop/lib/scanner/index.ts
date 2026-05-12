/**
 * Scanner module public API.
 */

export { createFrameSource, DEFAULT_SCANNER_CONFIG, makeScannerConfigFromSettings, computeScannerBitrate } from "./frame-source";
export type {
  FrameSource,
  FrameCallback,
  ErrorCallback,
  FrameSourceState,
  FrameSourceStatus,
  FrameSourceMetrics,
  FrameSourceBenchmarkSnapshot,
  FrameSourceCapabilities,
  FrameSourceStateCallback,
  ScannerConfig,
  ScannerStillCapture,
} from "./frame-source";

export type { Point } from "./types";

export {
  evaluateFrameMappingCompatibility,
  scalePointBetweenFrames,
  scalePointsBetweenFrames,
} from "./capture-mapping";
export type { FrameDimensions, FrameMappingCompatibility } from "./capture-mapping";
