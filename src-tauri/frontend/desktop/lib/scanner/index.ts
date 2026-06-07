export {
  createFrameSource,
  DEFAULT_SCANNER_CONFIG,
  makeScannerConfigFromSettings,
  computeScannerBitrate,
} from "./frame-source";

export type {
  ErrorCallback,
  FrameCallback,
  FrameSource,
  FrameSourceCapabilities,
  FrameSourceMetrics,
  FrameSourceState,
  FrameSourceStateCallback,
  FrameSourceStatus,
  ScannerConfig,
  ScannerStillCapture,
} from "./frame-source";

export {
  decodeFramePacketToRgba,
  parseFramePacket,
  FRAME_CODEC_I420,
  FRAME_CODEC_I420_TELEMETRY,
  FRAME_PACKET_HEADER_SIZE,
  FRAME_PACKET_TELEMETRY_SIZE,
} from "./frame-codec";

export type {
  DecodedRgbaFrame,
  FramePacketTelemetry,
  ParsedFramePacket,
} from "./frame-codec";

export {
  evaluateFrameMappingCompatibility,
  scalePointBetweenFrames,
  scalePointsBetweenFrames,
} from "./capture-mapping";

export type {
  FrameDimensions,
  FrameMappingCompatibility,
} from "./capture-mapping";

export type {Point} from "./types";
