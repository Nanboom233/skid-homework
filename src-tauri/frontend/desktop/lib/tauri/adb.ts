import {
  invokeTauriBinaryChannelCommand,
  invokeTauriCommand,
} from "./ipc";

export interface TauriAdbDevice {
  serial: string;
  name: string;
  state: string;
}

export interface TauriAdbConnectResult {
  serial: string;
  message: string;
}

export interface TauriAdbPairRequest {
  address: string;
  pairingCode: string;
}

export type ForwardRequest =
  | {
      mode: "add";
      serial: string;
      localPort: number;
      remoteSocketName: string;
    }
  | {
      mode: "remove";
      serial: string;
      localPort: number;
    };

export const listDevices = async (): Promise<TauriAdbDevice[]> => {
  return await invokeTauriCommand<TauriAdbDevice[]>("tauri_adb_list_devices");
};

export const pairDevice = async (
  request: TauriAdbPairRequest,
): Promise<string> => {
  return await invokeTauriCommand<string>("tauri_adb_pair", {request});
};

export const connectDevice = async (
  address: string,
): Promise<TauriAdbConnectResult> => {
  return await invokeTauriCommand<TauriAdbConnectResult>("tauri_adb_connect", {
    request: {address},
  });
};

export const screenshot = async (serial: string): Promise<Uint8Array> => {
  return await invokeTauriBinaryChannelCommand("tauri_adb_screenshot", {
    serial,
  });
};

export const push = async (
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

export const forward = async (request: ForwardRequest): Promise<string> => {
  if (request.mode === "add") {
    return await invokeTauriCommand<string>("tauri_adb_forward", {
      serial: request.serial,
      localPort: request.localPort,
      remoteSocketName: request.remoteSocketName,
    });
  }

  return await invokeTauriCommand<string>("tauri_adb_remove_forward", {
    serial: request.serial,
    localPort: request.localPort,
  });
};
