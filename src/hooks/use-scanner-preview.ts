import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import type {Dispatch, MutableRefObject, RefObject, SetStateAction} from "react";

import {useScannerStore} from "@/store/scanner-store";

export type PreviewOrientation = "landscape" | "portrait";

export interface UseScannerPreviewResult {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  orientation: PreviewOrientation;
  setOrientation: Dispatch<SetStateAction<PreviewOrientation>>;
  previewDimensions: {
    frameWidth: number;
    frameHeight: number;
    displayWidth: number;
    displayHeight: number;
  };
  orientationRef: MutableRefObject<PreviewOrientation>;
  getLatestFrame: () => ImageData | null;
  pushFrame: (frame: ImageData) => void;
  resetPreview: () => void;
  cancelRender: () => void;
}

export function useScannerPreview(): UseScannerPreviewResult {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const canvasContextRef = useRef<CanvasRenderingContext2D | null>(null);
  const frameBufferCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const frameBufferContextRef = useRef<CanvasRenderingContext2D | null>(null);
  const animationFrameRef = useRef<number | null>(null);
  const latestFrameRef = useRef<ImageData | null>(null);
  const latestFrameVersionRef = useRef(0);
  const renderedFrameVersionRef = useRef(0);
  const lastCanvasMetricEmitAtRef = useRef(0);
  const previewOrientationRef = useRef<PreviewOrientation>("landscape");

  const [previewOrientation, setPreviewOrientation] = useState<PreviewOrientation>("landscape");
  const [frameDimensions, setFrameDimensions] = useState({
    frameWidth: 0,
    frameHeight: 0,
  });

  const setPreviewDebug = useScannerStore((state) => state.setPreviewDebug);
  const status = useScannerStore((state) => state.status);
  const isStreaming = status === "streaming";

  const resolvePreviewCanvasContext = useCallback((canvas: HTMLCanvasElement) => {
    const cached = canvasContextRef.current;
    if (cached && cached.canvas === canvas) {
      return cached;
    }

    const context = canvas.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
    canvasContextRef.current = context;
    return context;
  }, []);

  const resolveFrameBufferContext = useCallback((canvas: HTMLCanvasElement) => {
    const cached = frameBufferContextRef.current;
    if (cached && cached.canvas === canvas) {
      return cached;
    }

    const context = canvas.getContext("2d", {
      alpha: false,
      desynchronized: true,
    });
    frameBufferContextRef.current = context;
    return context;
  }, []);

  const drawLatestFrameToCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    const frame = latestFrameRef.current;

    if (
      canvas
      && frame
      && renderedFrameVersionRef.current !== latestFrameVersionRef.current
    ) {
      renderedFrameVersionRef.current = latestFrameVersionRef.current;
      const drawStartedAt = performance.now();
      const ctx = resolvePreviewCanvasContext(canvas);
      if (ctx) {
        const isPortrait = previewOrientationRef.current === "portrait";
        const targetWidth = isPortrait ? frame.height : frame.width;
        const targetHeight = isPortrait ? frame.width : frame.height;

        if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
          canvas.width = targetWidth;
          canvas.height = targetHeight;
        }

        if (!isPortrait) {
          ctx.putImageData(frame, 0, 0);
        } else {
          if (!frameBufferCanvasRef.current) {
            frameBufferCanvasRef.current = document.createElement("canvas");
          }

          const frameBufferCanvas = frameBufferCanvasRef.current;
          if (frameBufferCanvas.width !== frame.width || frameBufferCanvas.height !== frame.height) {
            frameBufferCanvas.width = frame.width;
            frameBufferCanvas.height = frame.height;
          }

          const frameBufferContext = resolveFrameBufferContext(frameBufferCanvas);
          if (!frameBufferContext) {
            return;
          }

          frameBufferContext.putImageData(frame, 0, 0);
          ctx.save();
          ctx.translate(canvas.width, 0);
          ctx.rotate(Math.PI / 2);
          ctx.drawImage(frameBufferCanvas, 0, 0);
          ctx.restore();
        }

        const drawMs = performance.now() - drawStartedAt;
        const metricUpdatedAt = Date.now();
        if (metricUpdatedAt - lastCanvasMetricEmitAtRef.current >= 250) {
          lastCanvasMetricEmitAtRef.current = metricUpdatedAt;
          setPreviewDebug({
            canvasDrawMs: Number(drawMs.toFixed(1)),
            updatedAt: metricUpdatedAt,
          });
        }
      }
    }
  }, [
    resolveFrameBufferContext,
    resolvePreviewCanvasContext,
    setPreviewDebug,
  ]);

  const schedulePreviewRenderRef = useRef<() => void>(null!);
  const schedulePreviewRender = useCallback(() => {
    if (animationFrameRef.current !== null) {
      return;
    }

    animationFrameRef.current = requestAnimationFrame(() => {
      animationFrameRef.current = null;
      drawLatestFrameToCanvas();

      if (renderedFrameVersionRef.current !== latestFrameVersionRef.current) {
        schedulePreviewRenderRef.current();
      }
    });
  }, [drawLatestFrameToCanvas]);
  useEffect(() => {
    schedulePreviewRenderRef.current = schedulePreviewRender;
  });

  const cancelRender = useCallback(() => {
    if (animationFrameRef.current !== null) {
      cancelAnimationFrame(animationFrameRef.current);
      animationFrameRef.current = null;
    }
  }, []);

  const resetPreview = useCallback(() => {
    latestFrameRef.current = null;
    latestFrameVersionRef.current = 0;
    renderedFrameVersionRef.current = 0;
    lastCanvasMetricEmitAtRef.current = 0;
    setFrameDimensions((current) => {
      if (current.frameWidth === 0 && current.frameHeight === 0) {
        return current;
      }

      return {
        frameWidth: 0,
        frameHeight: 0,
      };
    });
  }, []);

  const getLatestFrame = useCallback(() => {
    return latestFrameRef.current;
  }, []);

  const pushFrame = useCallback((frame: ImageData) => {
    latestFrameRef.current = frame;
    latestFrameVersionRef.current += 1;
    setFrameDimensions((current) => {
      if (current.frameWidth === frame.width && current.frameHeight === frame.height) {
        return current;
      }

      return {
        frameWidth: frame.width,
        frameHeight: frame.height,
      };
    });
    schedulePreviewRender();
  }, [schedulePreviewRender]);

  useEffect(() => {
    previewOrientationRef.current = previewOrientation;
    renderedFrameVersionRef.current = 0;
    if (isStreaming) {
      schedulePreviewRender();
    }
  }, [isStreaming, previewOrientation, schedulePreviewRender]);

  useEffect(() => cancelRender, [cancelRender]);

  const previewDimensions = useMemo(() => {
    const frameWidth = frameDimensions.frameWidth;
    const frameHeight = frameDimensions.frameHeight;

    return {
      frameWidth,
      frameHeight,
      displayWidth: previewOrientation === "portrait" ? frameHeight : frameWidth,
      displayHeight: previewOrientation === "portrait" ? frameWidth : frameHeight,
    };
  }, [frameDimensions.frameHeight, frameDimensions.frameWidth, previewOrientation]);

  return {
    canvasRef,
    orientation: previewOrientation,
    setOrientation: setPreviewOrientation,
    previewDimensions,
    orientationRef: previewOrientationRef,
    getLatestFrame,
    pushFrame,
    resetPreview,
    cancelRender,
  };
}
