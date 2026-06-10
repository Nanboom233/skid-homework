import {push} from "../tauri/adb";
import {
  invokeTauriBinaryChannelCommand,
  invokeTauriCommand,
  normalizeTauriRawChannelPayload,
  type TauriRawChannelPayload,
} from "../tauri/ipc";
import {isTauri} from "../tauri/platform";

export interface CameraServerArtifact {
  path: string;
  source: string;
}

export interface DecodeStreamHandle {
  frameChannel: unknown;
  statusChannel: unknown;
  dispose: () => void;
}

export interface DecodeStreamLifecycleEvent {
  state: "starting" | "connected" | "connecting" | "reconnecting" | "ready" | "error" | "stopped";
  detail: string;
  recoverable: boolean;
  reconnectAttempt: number;
}

export interface StillPayload {
  mimeType: "image/jpeg";
  bytes: Uint8Array;
  transport: "device-file-channel" | "forwarded-stream-channel";
}

export interface DeployServerRequest {
  serial: string;
  remotePath: string;
  localPath?: string;
}

export interface StartServerRequest {
  serial: string;
  classpath: string;
  mainClass: string;
  serverArgs: string[];
}

export interface CaptureStillRequest {
  serial: string;
  classpath: string;
  socketName: string;
}

const resolveServerArtifact = async (): Promise<CameraServerArtifact> => {
  return await invokeTauriCommand<CameraServerArtifact>("scanner_camera_server_artifact");
};

export const deployServer = async ({
  serial,
  remotePath,
  localPath,
}: DeployServerRequest): Promise<string> => {
  const sourcePath = localPath ?? (await resolveServerArtifact()).path;
  return await push(serial, sourcePath, remotePath);
};

const startLogTailer = async (serial: string): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_adb_start_log_tailer", {serial});
};

const stopLogTailer = async (): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_adb_stop_log_tailer");
};

export const startServer = async ({
  serial,
  classpath,
  mainClass,
  serverArgs,
}: StartServerRequest): Promise<void> => {
  // The log tailer is diagnostic-only; transport readiness is owned by the decoder handshake.
  startLogTailer(serial).catch(() => {});

  try {
    await invokeTauriCommand<string>("tauri_adb_start_server", {
      serial,
      classpath,
      mainClass,
      serverArgs,
    });
  } catch (error) {
    stopLogTailer().catch(() => {});
    throw error;
  }
};

export const stopServer = async (serial: string): Promise<void> => {
  let stopError: unknown = null;

  try {
    await invokeTauriCommand<string>("tauri_adb_stop_server", {serial});
  } catch (error) {
    stopError = error;
  }

  try {
    await stopLogTailer();
  } catch {
    // Log tailer cleanup is best-effort; preserve the primary stop failure.
  }

  if (stopError) {
    throw stopError instanceof Error ? stopError : new Error(String(stopError));
  }
};

export const captureStill = async ({
  serial,
  classpath,
  socketName,
}: CaptureStillRequest): Promise<StillPayload> => {
  const bytes = await invokeTauriBinaryChannelCommand("tauri_adb_capture_still", {
    serial,
    classpath,
    socketName,
  });
  return {
    mimeType: "image/jpeg",
    bytes,
    transport: "device-file-channel",
  };
};

export const captureStillStream = async (
  port: number,
): Promise<StillPayload> => {
  const bytes = await invokeTauriBinaryChannelCommand("tauri_adb_capture_still_stream", {
    port,
  });
  return {
    mimeType: "image/jpeg",
    bytes,
    transport: "forwarded-stream-channel",
  };
};

export const startDecodeStream = async (
  port: number,
  onFrame: (framePacket: Uint8Array) => void,
  onLifecycleEvent: (event: DecodeStreamLifecycleEvent) => void,
  options?: {
    maxPreviewWidth?: number;
    maxPreviewHeight?: number;
  },
): Promise<DecodeStreamHandle> => {
  if (!isTauri()) {
    throw new Error("Tauri decoded frame streaming is only available in Tauri desktop builds.");
  }

  const {invoke, Channel} = await import("@tauri-apps/api/core");
  let latestFramePacket: TauriRawChannelPayload | null = null;
  let frameDispatchScheduled = false;
  let disposed = false;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;

  const clearScheduledDispatch = (): void => {
    if (timeoutId !== null) {
      clearTimeout(timeoutId);
      timeoutId = null;
    }
    frameDispatchScheduled = false;
  };

  const flushLatestFrame = (): void => {
    frameDispatchScheduled = false;
    timeoutId = null;

    if (disposed) {
      latestFramePacket = null;
      return;
    }

    const framePacket = latestFramePacket;
    latestFramePacket = null;
    if (framePacket === null) {
      return;
    }

    onFrame(normalizeTauriRawChannelPayload(framePacket));

    if (latestFramePacket !== null) {
      scheduleLatestFrameDispatch();
    }
  };

  function scheduleLatestFrameDispatch(): void {
    if (disposed || frameDispatchScheduled) {
      return;
    }

    frameDispatchScheduled = true;

    if (typeof queueMicrotask === "function") {
      queueMicrotask(() => {
        flushLatestFrame();
      });
      return;
    }

    timeoutId = setTimeout(() => {
      flushLatestFrame();
    }, 0);
  }

  const frameChannel = new Channel<TauriRawChannelPayload>((framePacket) => {
    if (disposed) {
      return;
    }

    latestFramePacket = framePacket;
    scheduleLatestFrameDispatch();
  });
  const statusChannel = new Channel<DecodeStreamLifecycleEvent>((event) => {
    if (disposed) {
      return;
    }

    onLifecycleEvent(event);
  });

  await invoke<void>("tauri_scanner_start_stream", {
    port,
    frameChannel,
    statusChannel,
    maxPreviewWidth: options?.maxPreviewWidth,
    maxPreviewHeight: options?.maxPreviewHeight,
  });

  return {
    frameChannel,
    statusChannel,
    dispose: () => {
      disposed = true;
      latestFramePacket = null;
      clearScheduledDispatch();
    },
  };
};

export const stopDecodeStream = async (): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_scanner_stop_stream");
};
