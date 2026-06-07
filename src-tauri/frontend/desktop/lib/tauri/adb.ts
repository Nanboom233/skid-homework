import {isTauri} from "./platform";

export interface TauriAdbDevice {
  serial: string;
  name: string;
  state: string;
}

export interface TauriAdbConnectResult {
  serial: string;
  message: string;
}

export interface TauriCameraServerArtifact {
  path: string;
  source: string;
}

export interface TauriDecodeStreamHandle {
  frameChannel: unknown;
  statusChannel: unknown;
  dispose: () => void;
}

export interface TauriDecodeStreamLifecycleEvent {
  state: "starting" | "connected" | "connecting" | "reconnecting" | "ready" | "error" | "stopped";
  detail: string;
  recoverable: boolean;
  reconnectAttempt: number;
}

export interface TauriAdbPairRequest {
  address: string;
  pairingCode: string;
}

export interface TauriAdbStillPayload {
  mimeType: "image/jpeg";
  bytes: Uint8Array;
  transport: "device-file-channel" | "forwarded-stream-channel";
}

type TauriRawChannelPayload = string | ArrayBuffer | Uint8Array | number[];

const invokeTauriCommand = async <T>(
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> => {
  if (!isTauri()) {
    throw new Error("Native ADB is only available in Tauri desktop builds.");
  }

  const { invoke } = await import("@tauri-apps/api/core");
  return await invoke<T>(command, payload);
};

const decodeBase64ToUint8Array = (base64: string): Uint8Array => {
  const normalized = base64.replace(/\s+/g, "");
  const binary = atob(normalized);
  const bytes = new Uint8Array(binary.length);

  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }

  return bytes;
};

const normalizeTauriRawChannelPayload = (payload: TauriRawChannelPayload): Uint8Array => {
  if (typeof payload === "string") {
    return decodeBase64ToUint8Array(payload);
  }

  if (payload instanceof ArrayBuffer) {
    return new Uint8Array(payload);
  }

  if (payload instanceof Uint8Array) {
    return payload;
  }

  if (Array.isArray(payload)) {
    return Uint8Array.from(payload);
  }

  throw new Error("Invalid binary payload from Tauri channel.");
};

const invokeTauriBinaryChannelCommand = async (
  command: string,
  payload?: Record<string, unknown>,
  channelKey: string = "payloadChannel",
): Promise<Uint8Array> => {
  if (!isTauri()) {
    throw new Error("Native binary channel IPC is only available in Tauri desktop builds.");
  }

  const { invoke, Channel } = await import("@tauri-apps/api/core");

  return await new Promise<Uint8Array>((resolve, reject) => {
    let settled = false;

    const settleResolve = (bytes: Uint8Array): void => {
      if (settled) {
        return;
      }

      settled = true;
      resolve(bytes);
    };

    const settleReject = (error: unknown): void => {
      if (settled) {
        return;
      }

      settled = true;
      reject(error instanceof Error ? error : new Error(String(error)));
    };

    const payloadChannel = new Channel<TauriRawChannelPayload>((message) => {
      try {
        settleResolve(normalizeTauriRawChannelPayload(message));
      } catch (error) {
        settleReject(error);
      }
    });

    void invoke<void>(command, {
      ...(payload ?? {}),
      [channelKey]: payloadChannel,
    }).catch((error) => {
      settleReject(error);
    });
  });
};

export const listTauriAdbDevices = async (): Promise<TauriAdbDevice[]> => {
  return await invokeTauriCommand<TauriAdbDevice[]>("tauri_adb_list_devices");
};

export const pairTauriAdbDevice = async (
  request: TauriAdbPairRequest,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_pair", { request });
};

export const connectTauriAdbDevice = async (
  address: string,
): Promise<TauriAdbConnectResult> => {
  return await invokeTauriCommand<TauriAdbConnectResult>("tauri_adb_connect", {
    request: { address },
  });
};

export const captureTauriAdbScreenshot = async (
  serial: string,
): Promise<Uint8Array> => {
  return await invokeTauriBinaryChannelCommand("tauri_adb_screenshot", {
    serial,
  });
};

// --- Scanner camera-server transport ---

export const resolveTauriCameraServerArtifact = async (): Promise<TauriCameraServerArtifact> => {
  return await invokeTauriCommand<TauriCameraServerArtifact>("scanner_camera_server_artifact");
};

export const pushTauriAdbFile = async (
  serial: string,
  localPath: string,
  remotePath: string,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_push", {
    serial,
    localPath,
    remotePath,
  });
};

export const forwardTauriAdbPort = async (
  serial: string,
  localPort: number,
  remoteSocketName: string,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_forward", {
    serial,
    localPort,
    remoteSocketName,
  });
};

export const removeForwardTauriAdbPort = async (
  serial: string,
  localPort: number,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_remove_forward", {
    serial,
    localPort,
  });
};

export const pushTauriCameraServer = async (
  serial: string,
  remotePath: string,
): Promise<string> => {
  const artifact = await resolveTauriCameraServerArtifact();
  return await pushTauriAdbFile(serial, artifact.path, remotePath);
};

export const startTauriAdbServer = async (
  serial: string,
  classpath: string,
  mainClass: string,
  serverArgs: string[],
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_start_server", {
    serial,
    classpath,
    mainClass,
    serverArgs,
  });
};

export const stopTauriAdbServer = async (
  serial: string,
  classpath: string,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_stop_server", {
    serial,
    classpath,
  });
};

export const awaitTauriAdbServerReady = async (
  serial: string,
  timeoutMs: number,
): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_adb_await_server_ready", {
    serial,
    timeoutMs,
  });
};

export const startTauriAdbLogTailer = async (serial: string): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_adb_start_log_tailer", {serial});
};

export const stopTauriAdbLogTailer = async (): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_adb_stop_log_tailer");
};

export const captureTauriAdbStill = async (
  serial: string,
  classpath: string,
  socketName: string,
): Promise<TauriAdbStillPayload> => {
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

export const captureTauriAdbStillStream = async (
  port: number,
): Promise<TauriAdbStillPayload> => {
  const bytes = await invokeTauriBinaryChannelCommand("tauri_adb_capture_still_stream", {
    port,
  });
  return {
    mimeType: "image/jpeg",
    bytes,
    transport: "forwarded-stream-channel",
  };
};

export const startTauriDecodeStream = async (
  port: number,
  onFrame: (framePacket: Uint8Array) => void,
  onLifecycleEvent: (event: TauriDecodeStreamLifecycleEvent) => void,
  options?: {
    maxPreviewWidth?: number;
    maxPreviewHeight?: number;
  },
): Promise<TauriDecodeStreamHandle> => {
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
  const statusChannel = new Channel<TauriDecodeStreamLifecycleEvent>((event) => {
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

export const stopTauriDecodeStream = async (): Promise<void> => {
  return await invokeTauriCommand<void>("tauri_scanner_stop_stream");
};
