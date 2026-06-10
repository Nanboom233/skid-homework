import {isTauri} from "./platform";

export type TauriRawChannelPayload = string | ArrayBuffer | Uint8Array | number[];

export const invokeTauriCommand = async <T>(
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> => {
  if (!isTauri()) {
    throw new Error("Native Tauri commands are only available in Tauri desktop builds.");
  }

  const {invoke} = await import("@tauri-apps/api/core");
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

export const normalizeTauriRawChannelPayload = (
  payload: TauriRawChannelPayload,
): Uint8Array => {
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

export const invokeTauriBinaryChannelCommand = async (
  command: string,
  payload?: Record<string, unknown>,
  channelKey: string = "payloadChannel",
): Promise<Uint8Array> => {
  if (!isTauri()) {
    throw new Error("Native binary channel IPC is only available in Tauri desktop builds.");
  }

  const {invoke, Channel} = await import("@tauri-apps/api/core");

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
