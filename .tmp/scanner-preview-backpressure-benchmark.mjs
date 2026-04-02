import { Worker, isMainThread, parentPort, workerData } from "node:worker_threads";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";

const FRAME_CODEC_I420_TELEMETRY = 4;
const FRAME_PACKET_HEADER_SIZE = 9;
const FRAME_PACKET_TELEMETRY_SIZE = 12;
const DEFAULT_WIDTH = 640;
const DEFAULT_HEIGHT = 360;
const DEFAULT_FPS = 30;
const DEFAULT_DURATION_MS = 8000;
const DEFAULT_DOWNSTREAM_MS = 36;
const SAMPLE_DIGITS = 3;

const IS_LITTLE_ENDIAN = new Uint8Array(new Uint32Array([0x11223344]).buffer)[0] === 0x44;
const LITTLE_ENDIAN_RGBA_ALPHA = 0xff << 24;
const BIG_ENDIAN_RGBA_ALPHA = 0xff;
const Y_TO_RGB_LUT = new Int32Array(256);
const U_TO_BLUE_LUT = new Int32Array(256);
const U_TO_GREEN_LUT = new Int32Array(256);
const V_TO_RED_LUT = new Int32Array(256);
const V_TO_GREEN_LUT = new Int32Array(256);

for (let value = 0; value < 256; value += 1) {
  const luma = Math.max(0, value - 16);
  const chroma = value - 128;
  Y_TO_RGB_LUT[value] = 298 * luma;
  U_TO_BLUE_LUT[value] = 516 * chroma;
  U_TO_GREEN_LUT[value] = -100 * chroma;
  V_TO_RED_LUT[value] = 409 * chroma;
  V_TO_GREEN_LUT[value] = -208 * chroma;
}

const parseArgs = (argv) => {
  const options = {
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
    fps: DEFAULT_FPS,
    durationMs: DEFAULT_DURATION_MS,
    downstreamMs: DEFAULT_DOWNSTREAM_MS,
    json: null,
    md: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case "--width":
        options.width = Number(argv[++index]);
        break;
      case "--height":
        options.height = Number(argv[++index]);
        break;
      case "--fps":
        options.fps = Number(argv[++index]);
        break;
      case "--duration-ms":
        options.durationMs = Number(argv[++index]);
        break;
      case "--downstream-ms":
        options.downstreamMs = Number(argv[++index]);
        break;
      case "--json":
        options.json = argv[++index] ?? null;
        break;
      case "--md":
        options.md = argv[++index] ?? null;
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
      default:
        throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!Number.isInteger(options.width) || options.width <= 0 || (options.width & 1) !== 0) {
    throw new Error("--width must be a positive even integer.");
  }
  if (!Number.isInteger(options.height) || options.height <= 0 || (options.height & 1) !== 0) {
    throw new Error("--height must be a positive even integer.");
  }
  if (!Number.isFinite(options.fps) || options.fps <= 0) {
    throw new Error("--fps must be > 0.");
  }
  if (!Number.isFinite(options.durationMs) || options.durationMs < 1000) {
    throw new Error("--duration-ms must be >= 1000.");
  }
  if (!Number.isFinite(options.downstreamMs) || options.downstreamMs < 0) {
    throw new Error("--downstream-ms must be >= 0.");
  }

  return options;
};

const printHelp = () => {
  console.log(
    "benchmark-preview-backpressure.mjs [--width 640] [--height 360] [--fps 30] [--duration-ms 8000] [--downstream-ms 36] [--json <path>] [--md <path>]",
  );
};

const roundMetric = (value, digits = SAMPLE_DIGITS) => {
  if (!Number.isFinite(value)) {
    return 0;
  }

  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
};

const summarizeSamples = (values, digits = SAMPLE_DIGITS) => {
  if (values.length === 0) {
    return {
      sampleCount: 0,
      average: null,
      p95: null,
      max: null,
      min: null,
    };
  }

  const sorted = [...values].sort((left, right) => left - right);
  const p95Index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * 0.95) - 1));
  const total = values.reduce((sum, value) => sum + value, 0);

  return {
    sampleCount: values.length,
    average: roundMetric(total / values.length, digits),
    p95: roundMetric(sorted[p95Index], digits),
    max: roundMetric(sorted[sorted.length - 1], digits),
    min: roundMetric(sorted[0], digits),
  };
};

const clampByte = (value) => {
  if (value < 0) return 0;
  if (value > 255) return 255;
  return value;
};

const packRgbaFromYuv = (y, redContribution, greenContribution, blueContribution) => {
  const base = Y_TO_RGB_LUT[y];
  const red = clampByte((base + redContribution + 128) >> 8);
  const green = clampByte((base + greenContribution + 128) >> 8);
  const blue = clampByte((base + blueContribution + 128) >> 8);

  if (IS_LITTLE_ENDIAN) {
    return LITTLE_ENDIAN_RGBA_ALPHA | (blue << 16) | (green << 8) | red;
  }

  return (red << 24) | (green << 16) | (blue << 8) | BIG_ENDIAN_RGBA_ALPHA;
};

const resolveRgbaTarget = (pixelCount, targetRgba) => {
  const byteLength = pixelCount * 4;

  if (
    targetRgba
    && targetRgba.length === byteLength
    && (targetRgba.byteOffset & 0x03) === 0
  ) {
    return {
      rgba: targetRgba,
      rgba32: new Uint32Array(targetRgba.buffer, targetRgba.byteOffset, pixelCount),
    };
  }

  const rgba = new Uint8ClampedArray(byteLength);
  return {
    rgba,
    rgba32: new Uint32Array(rgba.buffer, rgba.byteOffset, pixelCount),
  };
};

const decodeI420ToRgba = (payload, width, height, targetRgba) => {
  const lumaSize = width * height;
  const chromaWidth = width >> 1;
  const chromaHeight = height >> 1;
  const chromaSize = chromaWidth * chromaHeight;
  const expectedSize = lumaSize + chromaSize * 2;

  if (payload.length !== expectedSize) {
    throw new Error(`Invalid I420 payload size: expected ${expectedSize}, got ${payload.length}.`);
  }

  const yPlane = payload.subarray(0, lumaSize);
  const uPlane = payload.subarray(lumaSize, lumaSize + chromaSize);
  const vPlane = payload.subarray(lumaSize + chromaSize, expectedSize);
  const { rgba, rgba32 } = resolveRgbaTarget(lumaSize, targetRgba);

  for (let row = 0; row < height; row += 2) {
    const yRow0 = row * width;
    const yRow1 = yRow0 + width;
    const chromaRow = (row >> 1) * chromaWidth;

    for (let col = 0; col < width; col += 2) {
      const chromaIndex = chromaRow + (col >> 1);
      const u = uPlane[chromaIndex];
      const v = vPlane[chromaIndex];
      const blueContribution = U_TO_BLUE_LUT[u];
      const greenContribution = U_TO_GREEN_LUT[u] + V_TO_GREEN_LUT[v];
      const redContribution = V_TO_RED_LUT[v];

      const pixel0 = yRow0 + col;
      rgba32[pixel0] = packRgbaFromYuv(yPlane[pixel0], redContribution, greenContribution, blueContribution);
      rgba32[pixel0 + 1] = packRgbaFromYuv(yPlane[pixel0 + 1], redContribution, greenContribution, blueContribution);

      const pixel2 = yRow1 + col;
      rgba32[pixel2] = packRgbaFromYuv(yPlane[pixel2], redContribution, greenContribution, blueContribution);
      rgba32[pixel2 + 1] = packRgbaFromYuv(yPlane[pixel2 + 1], redContribution, greenContribution, blueContribution);
    }
  }

  return rgba;
};

const parseFramePacket = (buffer) => {
  const packet = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
  if (packet.byteLength < FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE) {
    throw new Error("Packet too short.");
  }

  const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
  const codec = view.getUint8(0);
  if (codec !== FRAME_CODEC_I420_TELEMETRY) {
    throw new Error(`Unsupported codec: ${codec}`);
  }

  const width = view.getUint32(1, false);
  const height = view.getUint32(5, false);
  const sentAtEpochMs = Number(view.getBigUint64(FRAME_PACKET_HEADER_SIZE, false));
  const sequence = view.getUint32(FRAME_PACKET_HEADER_SIZE + 8, false);
  const payloadOffset = FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE;
  return {
    width,
    height,
    sentAtEpochMs,
    sequence,
    payload: packet.subarray(payloadOffset),
  };
};

const decodeFramePacketToRgba = (buffer, targetRgba) => {
  const packet = parseFramePacket(buffer);
  return {
    width: packet.width,
    height: packet.height,
    sequence: packet.sequence,
    sentAtEpochMs: packet.sentAtEpochMs,
    rgba: decodeI420ToRgba(packet.payload, packet.width, packet.height, targetRgba),
  };
};

const busyWait = (durationMs) => {
  if (durationMs <= 0) {
    return;
  }

  const deadline = performance.now() + durationMs;
  while (performance.now() < deadline) {
    // Intentional busy wait to simulate preview draw / CV work on the renderer main thread.
  }
};

const ensureParentDir = async (filePath) => {
  if (!filePath) {
    return;
  }

  await mkdir(dirname(resolve(filePath)), { recursive: true });
};

const createI420Payload = (width, height) => {
  const lumaSize = width * height;
  const chromaSize = lumaSize >> 2;
  const payload = new Uint8Array(lumaSize + chromaSize * 2);

  for (let row = 0; row < height; row += 1) {
    const rowOffset = row * width;
    for (let col = 0; col < width; col += 1) {
      payload[rowOffset + col] = (row * 3 + col * 5) & 0xff;
    }
  }

  const uOffset = lumaSize;
  const vOffset = lumaSize + chromaSize;
  for (let index = 0; index < chromaSize; index += 1) {
    payload[uOffset + index] = 96 + (index % 32);
    payload[vOffset + index] = 160 - (index % 32);
  }

  return payload;
};

const createPacketTemplate = (width, height, payload) => {
  const packet = new Uint8Array(FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE + payload.byteLength);
  const view = new DataView(packet.buffer);
  view.setUint8(0, FRAME_CODEC_I420_TELEMETRY);
  view.setUint32(1, width, false);
  view.setUint32(5, height, false);
  packet.set(payload, FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE);
  return packet;
};

const writePacketTelemetry = (packet, sentAtEpochMs, sequence) => {
  const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
  view.setBigUint64(FRAME_PACKET_HEADER_SIZE, BigInt(sentAtEpochMs), false);
  view.setUint32(FRAME_PACKET_HEADER_SIZE + 8, sequence, false);
};

const runProducer = () => {
  const { width, height, fps, durationMs } = workerData;
  const payload = createI420Payload(width, height);
  const template = createPacketTemplate(width, height, payload);
  const frameIntervalMs = 1000 / fps;
  let sequence = 0;
  const startedAt = performance.now();

  const sendPacket = () => {
    const elapsedMs = performance.now() - startedAt;
    if (elapsedMs >= durationMs) {
      parentPort.postMessage({ type: "done", producedFrames: sequence });
      return;
    }

    const packet = new Uint8Array(template);
    writePacketTelemetry(packet, Date.now(), sequence);
    parentPort.postMessage({ type: "packet", sequence, packet: packet.buffer }, [packet.buffer]);
    sequence += 1;

    const nextDelayMs = Math.max(0, frameIntervalMs - ((performance.now() - startedAt) - (sequence * frameIntervalMs)));
    setTimeout(sendPacket, nextDelayMs);
  };

  sendPacket();
};

const waitFor = async (predicate, timeoutMs = 5000) => {
  const startedAt = performance.now();
  while (!predicate()) {
    if (performance.now() - startedAt >= timeoutMs) {
      throw new Error("Timed out while waiting for synthetic benchmark to drain.");
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
};

const runMode = async (mode, options) => {
  const worker = new Worker(new URL(import.meta.url), {
    workerData: {
      role: "producer",
      width: options.width,
      height: options.height,
      fps: options.fps,
      durationMs: options.durationMs,
    },
  });

  const ipcSamples = [];
  const decodeSamples = [];
  let decodeTarget = null;
  let firstProcessedAt = null;
  let lastProcessedAt = null;
  let processedFrames = 0;
  let producedFrames = 0;
  let droppedFrames = 0;
  let latestQueuedPacket = null;
  let drainScheduled = false;
  let producerDone = false;

  const processPacket = (packetBuffer) => {
    const decodeStartedAt = performance.now();
    const decoded = decodeFramePacketToRgba(packetBuffer, decodeTarget);
    decodeTarget = decoded.rgba;
    const decodeMs = performance.now() - decodeStartedAt;
    busyWait(options.downstreamMs);

    const processedAt = Date.now();
    processedFrames += 1;
    firstProcessedAt ??= processedAt;
    lastProcessedAt = processedAt;
    ipcSamples.push(Math.max(0, processedAt - decoded.sentAtEpochMs));
    decodeSamples.push(decodeMs);
  };

  const scheduleLatestDrain = () => {
    if (drainScheduled) {
      return;
    }

    drainScheduled = true;
    setTimeout(() => {
      drainScheduled = false;
      const packetBuffer = latestQueuedPacket;
      latestQueuedPacket = null;
      if (packetBuffer) {
        processPacket(packetBuffer);
      }
      if (latestQueuedPacket !== null) {
        scheduleLatestDrain();
        return;
      }
      if (producerDone) {
        void worker.terminate();
      }
    }, 0);
  };

  worker.on("message", (message) => {
    if (message.type === "packet") {
      producedFrames = Math.max(producedFrames, message.sequence + 1);
      if (mode === "eager") {
        processPacket(message.packet);
        return;
      }

      if (latestQueuedPacket !== null) {
        droppedFrames += 1;
      }
      latestQueuedPacket = message.packet;
      scheduleLatestDrain();
      return;
    }

    if (message.type === "done") {
      producedFrames = Math.max(producedFrames, message.producedFrames);
      producerDone = true;
      if (mode === "eager") {
        void worker.terminate();
      } else if (!drainScheduled && latestQueuedPacket === null) {
        void worker.terminate();
      }
    }
  });

  worker.on("error", (error) => {
    throw error;
  });

  await waitFor(
    () => producerDone && !drainScheduled && latestQueuedPacket === null,
    options.durationMs + 10000,
  );

  const runtimeMs = firstProcessedAt !== null && lastProcessedAt !== null
    ? Math.max(0, lastProcessedAt - firstProcessedAt)
    : 0;
  const effectiveFps = processedFrames > 1 && runtimeMs > 0
    ? roundMetric(processedFrames / (runtimeMs / 1000), 2)
    : 0;

  return {
    mode,
    producedFrames,
    processedFrames,
    droppedFrames,
    effectiveFps,
    ipcMs: summarizeSamples(ipcSamples),
    decodeMs: summarizeSamples(decodeSamples),
  };
};

const formatMarkdown = (result) => {
  const eager = result.modes.find((mode) => mode.mode === "eager");
  const latestOnly = result.modes.find((mode) => mode.mode === "latest-only");
  const lines = [
    "# 2026-04-03 扫描预览背压调度基准",
    "",
    `- 状态: \`${result.status}\``,
    `- 生成时间: \`${result.generatedAt}\``,
    `- 场景: \`${result.scenario}\``,
    `- 输入: \`${result.input.width}x${result.input.height} @ ${result.input.fps}fps\``,
    `- 模拟下游成本: \`${result.input.downstreamMs}ms / frame\``,
    "",
    "## 结论",
    "",
    `- eager 平均 IPC: \`${eager?.ipcMs.average ?? "n/a"}\` ms`,
    `- latest-only 平均 IPC: \`${latestOnly?.ipcMs.average ?? "n/a"}\` ms`,
    `- eager P95 IPC: \`${eager?.ipcMs.p95 ?? "n/a"}\` ms`,
    `- latest-only P95 IPC: \`${latestOnly?.ipcMs.p95 ?? "n/a"}\` ms`,
    `- latest-only 丢弃的陈旧 packet: \`${latestOnly?.droppedFrames ?? 0}\``,
    "",
    "## 模式对比",
    "",
    "| Mode | Produced | Processed | Dropped | Effective FPS | IPC Avg | IPC P95 | Decode Avg |",
    "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
  ];

  for (const mode of result.modes) {
    lines.push(
      `| ${mode.mode} | ${mode.producedFrames} | ${mode.processedFrames} | ${mode.droppedFrames} | ${mode.effectiveFps} | ${mode.ipcMs.average ?? "n/a"} | ${mode.ipcMs.p95 ?? "n/a"} | ${mode.decodeMs.average ?? "n/a"} |`,
    );
  }

  return `${lines.join("\n")}\n`;
};

const main = async () => {
  const options = parseArgs(process.argv.slice(2));
  const modes = [
    await runMode("eager", options),
    await runMode("latest-only", options),
  ];

  const result = {
    generatedAt: new Date().toISOString(),
    status: "passed",
    scenario: "synthetic-backpressure",
    input: {
      width: options.width,
      height: options.height,
      fps: options.fps,
      durationMs: options.durationMs,
      downstreamMs: options.downstreamMs,
    },
    modes,
    notes: [
      "Producer runs on a separate worker thread to emulate Tauri channel callbacks continuing while the renderer main thread is busy.",
      "latest-only mode mirrors the new frame-source latest-packet drain gate that yields one macrotask before decode.",
    ],
  };

  if (options.json) {
    await ensureParentDir(options.json);
    await writeFile(resolve(options.json), `${JSON.stringify(result, null, 2)}\n`, "utf8");
  }

  if (options.md) {
    await ensureParentDir(options.md);
    await writeFile(resolve(options.md), formatMarkdown(result), "utf8");
  }

  console.log(JSON.stringify(result, null, 2));
};

if (!isMainThread && workerData?.role === "producer") {
  runProducer();
} else {
  main().catch((error) => {
    console.error(error instanceof Error ? error.stack ?? error.message : String(error));
    process.exit(1);
  });
}
