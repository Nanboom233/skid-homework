import type {Point} from "./document-detector";
import type {OrthogonalRotation} from "./image-data";
import type {ScannerPostProcessBackend} from "@/store/settings-store";

export interface ScannerPostProcessWorkerInitRequest {
  type: "init";
}

export interface ScannerPostProcessWorkerProcessRequest {
  type: "process";
  requestId: number;
  inputKind: "image-data" | "encoded-image";
  width?: number;
  height?: number;
  pixels?: ArrayBuffer;
  sourceBlob?: Blob;
  documentPoints: Point[] | null;
  outputRotation: OrthogonalRotation;
  imageEnhancement: boolean;
  colorMode: string;
  postprocessBackend: ScannerPostProcessBackend;
  spineFlattening: boolean;
}

export type ScannerPostProcessWorkerRequest =
  | ScannerPostProcessWorkerInitRequest
  | ScannerPostProcessWorkerProcessRequest;

export interface ScannerPostProcessWorkerReadyResponse {
  type: "ready";
}

export interface ScannerPostProcessWorkerResultResponse {
  type: "result";
  requestId: number;
  processingMs: number;
  decodeMs: number | null;
  refineMs: number | null;
  perspectiveMs: number | null;
  flattenMs: number | null;
  enhanceMs: number | null;
  modelMs: number | null;
  residualWarpMs: number | null;
  rotateMs: number | null;
  encodeMs: number;
  inputWidth: number;
  inputHeight: number;
  outputWidth: number;
  outputHeight: number;
  encodedMimeType: string;
  postprocessBackend: ScannerPostProcessBackend;
  modelId: string | null;
  controlGridShape: string | null;
  effectiveDocumentPoints: Point[] | null;
  refinementApplied: boolean;
  localFlatteningApplied: boolean;
  residualWarpApplied: boolean;
  residualWarpFallbackReason: string | null;
  encodedBytes: ArrayBuffer;
}

export interface ScannerPostProcessWorkerErrorResponse {
  type: "error";
  phase: "init" | "process" | "runtime";
  message: string;
  requestId?: number;
}

export type ScannerPostProcessWorkerResponse =
  | ScannerPostProcessWorkerReadyResponse
  | ScannerPostProcessWorkerResultResponse
  | ScannerPostProcessWorkerErrorResponse;
