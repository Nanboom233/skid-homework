package com.skidhomework.server;

import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaFormat;
import android.os.Bundle;
import android.os.SystemClock;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

/**
 * H.264 encoder variant that accepts legacy camera1 preview frames through
 * byte-buffer input instead of a Surface.
 */
public final class ByteBufferVideoEncoder implements PreviewStreamEncoder {

    private static final String MIME_TYPE = "video/avc";
    private static final int I_FRAME_INTERVAL = 2;
    private static final String KEY_PREPEND_SPS_PPS_TO_IDR_FRAMES = "prepend-sps-pps-to-idr-frames";
    private static final long STARTUP_SYNC_FRAME_RETRY_MS = 250L;
    private static final long INPUT_BUFFER_TIMEOUT_US = 0L;

    private final MediaCodec codec;
    private final SocketRelay relay;
    private final Consumer<StopReason> stopCallback;
    private final AtomicBoolean started = new AtomicBoolean(false);
    private final AtomicBoolean closed = new AtomicBoolean(false);
    private final AtomicBoolean firstFrameReported = new AtomicBoolean(false);
    private final CountDownLatch firstFrameLatch = new CountDownLatch(1);
    private final int width;
    private final int height;
    private final int inputColorFormat;
    private final int frameByteCount;
    private final byte[] conversionBuffer;

    private volatile boolean running;
    private Thread drainThread;
    private long startedAtMs = -1L;
    private final boolean forceSemiPlanarConversion;
    private int nalDiagCount = 0;
    private static final int NAL_DIAG_LIMIT = 5;

    public ByteBufferVideoEncoder(
            int width,
            int height,
            int bitrate,
            int framerate,
            SocketRelay relay,
            Consumer<StopReason> stopCallback
    ) throws IOException {
        this.width = width;
        this.height = height;
        this.relay = relay;
        this.stopCallback = stopCallback;
        this.frameByteCount = width * height * 3 / 2;

        codec = MediaCodec.createEncoderByType(MIME_TYPE);
        MediaCodecInfo.CodecCapabilities capabilities =
                codec.getCodecInfo().getCapabilitiesForType(MIME_TYPE);
        inputColorFormat = selectInputColorFormat(capabilities.colorFormats);
        conversionBuffer = new byte[frameByteCount];

        MediaFormat format = MediaFormat.createVideoFormat(MIME_TYPE, width, height);
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT, inputColorFormat);
        format.setInteger(MediaFormat.KEY_BIT_RATE, bitrate);
        format.setInteger(MediaFormat.KEY_FRAME_RATE, framerate);
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, I_FRAME_INTERVAL);
        format.setInteger(KEY_PREPEND_SPS_PPS_TO_IDR_FRAMES, 1);

        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE);

        System.out.println(
                "[Encoder] Using byte-buffer H.264 input format "
                        + describeColorFormat(inputColorFormat)
                        + " for "
                        + width
                        + "x"
                        + height
                        + "."
        );
        System.out.println("[EncoderDiag] Backend: Camera1 ByteBuffer (NV21)");
        System.out.println("[EncoderDiag] Codec: " + codec.getName());
        System.out.println("[EncoderDiag] Supported color formats: "
                + java.util.Arrays.toString(capabilities.colorFormats));
        System.out.println("[EncoderDiag] Selected input format: "
                + describeColorFormat(inputColorFormat) + " (" + inputColorFormat + ")");
        System.out.println("[EncoderDiag] Config: " + width + "x" + height
                + ", bitrate=" + bitrate + ", fps=" + framerate
                + ", frameByteCount=" + frameByteCount);

        // MTK codecs claim COLOR_FormatYUV420Planar but internally expect
        // semi-planar (NV12) layout. Detect and force NV12 conversion path.
        String codecName = codec.getCodecInfo().getCanonicalName().toLowerCase();
        this.forceSemiPlanarConversion = codecName.startsWith("c2.mtk")
                || codecName.contains(".mtk.");
        if (forceSemiPlanarConversion) {
            System.out.println("[EncoderDiag] MTK codec detected — forcing NV12 conversion path"
                    + " (ignoring claimed Planar support).");
        }
    }

    @Override
    public void start() {
        if (closed.get()) {
            throw new IllegalStateException("encoder has already been released");
        }
        if (!started.compareAndSet(false, true)) {
            return;
        }

        running = true;
        startedAtMs = SystemClock.elapsedRealtime();
        codec.start();
        requestSyncFrame("[Encoder] Requested startup sync frame for byte-buffer input.");

        drainThread = new Thread(this::drainOutputBuffers, "EncoderDrain");
        drainThread.setDaemon(true);
        drainThread.start();

        System.out.println("[Encoder] Started byte-buffer H.264 encoding.");
    }

    @Override
    public void awaitFirstFrame(long timeoutMs) throws InterruptedException, IOException {
        long deadlineMs = SystemClock.elapsedRealtime() + timeoutMs;
        requestSyncFrame("[Encoder] Requested first-frame sync frame after legacy camera start.");
        while (SystemClock.elapsedRealtime() < deadlineMs) {
            long remainingMs = deadlineMs - SystemClock.elapsedRealtime();
            long waitSliceMs = Math.min(remainingMs, STARTUP_SYNC_FRAME_RETRY_MS);
            if (firstFrameLatch.await(waitSliceMs, TimeUnit.MILLISECONDS)) {
                return;
            }

            if (!running) {
                throw new IOException("encoder stopped before producing the first frame");
            }

            requestSyncFrame("[Encoder] Startup frame still pending; requesting another sync frame.");
        }

        throw new IOException("encoder did not produce the first frame within " + timeoutMs + "ms");
    }

    public boolean queueNv21Frame(byte[] nv21Frame, long presentationTimeNs) {
        if (!running || nv21Frame == null) {
            return false;
        }

        if (nv21Frame.length < frameByteCount) {
            System.err.println(
                    "[Encoder] Ignoring undersized legacy preview frame: expected "
                            + frameByteCount
                            + " bytes but received "
                            + nv21Frame.length
                            + "."
            );
            return false;
        }

        try {
            int inputIndex = codec.dequeueInputBuffer(INPUT_BUFFER_TIMEOUT_US);
            if (inputIndex < 0) {
                return false;
            }

            ByteBuffer inputBuffer = codec.getInputBuffer(inputIndex);
            if (inputBuffer == null) {
                codec.queueInputBuffer(inputIndex, 0, 0, presentationTimeNs / 1000L, 0);
                return false;
            }

            convertNv21ToCodecInput(nv21Frame, conversionBuffer);

            // [EncoderDiag] Structured color pipeline diagnostic on first frame.
            if (!firstFrameReported.get()) {
                logColorPipelineDiagnostics(nv21Frame, conversionBuffer);
            }
            inputBuffer.clear();
            inputBuffer.put(conversionBuffer, 0, frameByteCount);
            codec.queueInputBuffer(
                    inputIndex,
                    0,
                    frameByteCount,
                    presentationTimeNs / 1000L,
                    0
            );
            return true;
        } catch (IllegalStateException e) {
            if (running) {
                System.err.println("[Encoder] Failed to queue legacy preview frame: " + e.getMessage());
                stopCallback.accept(
                        StopReason.encoderFailed(
                                "failed to queue legacy preview frame: " + e.getMessage()
                        )
                );
            }
            return false;
        }
    }

    @Override
    public void stop() {
        if (!closed.compareAndSet(false, true)) {
            return;
        }

        running = false;
        if (drainThread != null && Thread.currentThread() != drainThread) {
            drainThread.interrupt();
            try {
                drainThread.join(500);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        if (started.get()) {
            try {
                codec.stop();
            } catch (IllegalStateException e) {
                // Encoder may already be stopped or released during teardown.
            }
        }
        try {
            codec.release();
        } catch (IllegalStateException e) {
            // Encoder may already be released during teardown.
        }
    }

    private void drainOutputBuffers() {
        MediaCodec.BufferInfo bufferInfo = new MediaCodec.BufferInfo();

        try {
            while (running) {
                int outputIndex = codec.dequeueOutputBuffer(bufferInfo, 10_000);

                if (outputIndex >= 0) {
                    ByteBuffer outputBuffer = codec.getOutputBuffer(outputIndex);

                    if (outputBuffer != null && bufferInfo.size > 0) {
                        if (isFramePayload(bufferInfo)
                                && firstFrameReported.compareAndSet(false, true)) {
                            System.out.println(
                                    "[Encoder] First encoded legacy frame ready in "
                                            + (SystemClock.elapsedRealtime() - startedAtMs)
                                            + "ms."
                            );
                            firstFrameLatch.countDown();
                        }

                        byte[] nalData = new byte[bufferInfo.size];
                        outputBuffer.position(bufferInfo.offset);
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size);
                        outputBuffer.get(nalData);

                        // [EncoderDiag] Log first NAL unit metadata for diagnostics.
                        if (nalDiagCount < NAL_DIAG_LIMIT
                                && firstFrameReported.get()
                                && isFramePayload(bufferInfo)
                                && nalData.length > 0) {
                            nalDiagCount++;
                            int nalType = (nalData[0] & 0x1F);
                            System.out.println("[EncoderDiag] First NAL: type="
                                    + nalType
                                    + ", size=" + nalData.length);
                        }

                        try {
                            relay.sendNalUnit(nalData);
                        } catch (IOException e) {
                            System.err.println("[Encoder] Failed to send NAL unit: " + e.getMessage());
                            running = false;
                            break;
                        }
                    }

                    codec.releaseOutputBuffer(outputIndex, false);

                    if ((bufferInfo.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                        stopCallback.accept(StopReason.encoderEndOfStream());
                        running = false;
                        break;
                    }
                } else if (outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    MediaFormat newFormat = codec.getOutputFormat();
                    System.out.println("[Encoder] Output format changed: " + newFormat);
                }
            }
        } catch (IllegalStateException e) {
            if (running) {
                System.err.println("[Encoder] Encoder stopped unexpectedly: " + e.getMessage());
                stopCallback.accept(
                        StopReason.encoderFailed("encoder stopped unexpectedly: " + e.getMessage())
                );
            }
        } finally {
            running = false;
        }
    }

    private void convertNv21ToCodecInput(byte[] sourceNv21, byte[] targetBuffer) {
        int lumaByteCount = width * height;
        System.arraycopy(sourceNv21, 0, targetBuffer, 0, lumaByteCount);

        if (!forceSemiPlanarConversion
                && (inputColorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar
                    || inputColorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)) {
            // I420 path: de-interleave NV21 VU pairs → separate U + V planes.
            int chromaPlaneByteCount = lumaByteCount / 4;
            int uPlaneOffset = lumaByteCount;
            int vPlaneOffset = lumaByteCount + chromaPlaneByteCount;
            for (int index = 0; index < chromaPlaneByteCount; index++) {
                int sourceOffset = lumaByteCount + (index * 2);
                targetBuffer[uPlaneOffset + index] = sourceNv21[sourceOffset + 1];
                targetBuffer[vPlaneOffset + index] = sourceNv21[sourceOffset];
            }
            return;
        }

        // NV12 path: swap interleaved VU → UV pairs (semi-planar).
        for (int index = lumaByteCount; index < frameByteCount; index += 2) {
            targetBuffer[index] = sourceNv21[index + 1];
            targetBuffer[index + 1] = sourceNv21[index];
        }
    }

    /**
     * Log a structured color pipeline diagnostic block on the first encoded frame.
     *
     * <p>Covers: source/target format, conversion path, frame geometry,
     * spatial Y sampling, interleaved chroma input, separated chroma output,
     * and an explicit UV swap correctness check.
     */
    private void logColorPipelineDiagnostics(byte[] nv21Input, byte[] convertedOutput) {
        int lumaBytes = width * height;
        int chromaPlaneBytes = lumaBytes / 4;
        boolean isPlanar = (inputColorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar
                || inputColorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible);

        String conversionPath = isPlanar
                ? "NV21 → I420 (de-interleave VU → separate U + V planes)"
                : "NV21 → NV12 (swap interleaved VU → UV pairs)";

        System.out.println("[EncoderDiag] ─── Color Pipeline Summary ───");
        System.out.println("[EncoderDiag] Source format: NV21 (Y plane + interleaved VU)");
        System.out.println("[EncoderDiag] Target format: " + describeColorFormat(inputColorFormat)
                + " (0x" + Integer.toHexString(inputColorFormat) + ")");
        System.out.println("[EncoderDiag] Conversion path: " + conversionPath);
        System.out.println("[EncoderDiag] Frame: " + width + "x" + height
                + ", lumaBytes=" + lumaBytes
                + ", chromaPlaneBytes=" + chromaPlaneBytes
                + ", totalBytes=" + frameByteCount);

        // Spatial Y sampling: sample at multiple positions to detect blank/constant frames.
        int[] yPositions = {
                0,
                width / 4,
                width / 2,
                (width * 3) / 4,
                lumaBytes / 4,
                lumaBytes / 2,
        };
        StringBuilder ySamples = new StringBuilder("[EncoderDiag] NV21 Y spatial sample: ");
        for (int pos : yPositions) {
            if (pos >= 0 && pos < lumaBytes && pos < nv21Input.length) {
                ySamples.append(String.format("@%d=%02X ", pos, nv21Input[pos] & 0xFF));
            }
        }
        System.out.println(ySamples.toString().trim());

        // NV21 interleaved chroma input: first 8 bytes (4 VU pairs).
        StringBuilder chromaIn = new StringBuilder("[EncoderDiag] NV21 chroma input (first 8 VU bytes): ");
        for (int i = lumaBytes; i < Math.min(lumaBytes + 8, nv21Input.length); i++) {
            chromaIn.append(String.format("%02X ", nv21Input[i] & 0xFF));
        }
        System.out.println(chromaIn.toString().trim());

        // Converted output chroma sampling.
        if (isPlanar) {
            int uPlaneOffset = lumaBytes;
            int vPlaneOffset = lumaBytes + chromaPlaneBytes;

            StringBuilder uOut = new StringBuilder("[EncoderDiag] Converted U plane (first 4 bytes @" + uPlaneOffset + "): ");
            for (int i = 0; i < Math.min(4, chromaPlaneBytes); i++) {
                if (uPlaneOffset + i < convertedOutput.length) {
                    uOut.append(String.format("%02X ", convertedOutput[uPlaneOffset + i] & 0xFF));
                }
            }
            System.out.println(uOut.toString().trim());

            StringBuilder vOut = new StringBuilder("[EncoderDiag] Converted V plane (first 4 bytes @" + vPlaneOffset + "): ");
            for (int i = 0; i < Math.min(4, chromaPlaneBytes); i++) {
                if (vPlaneOffset + i < convertedOutput.length) {
                    vOut.append(String.format("%02X ", convertedOutput[vPlaneOffset + i] & 0xFF));
                }
            }
            System.out.println(vOut.toString().trim());

            // UV swap verification: NV21 stores V,U interleaved; I420 stores U plane, then V plane.
            if (lumaBytes + 1 < nv21Input.length
                    && uPlaneOffset < convertedOutput.length
                    && vPlaneOffset < convertedOutput.length) {
                int srcV = nv21Input[lumaBytes] & 0xFF;
                int srcU = nv21Input[lumaBytes + 1] & 0xFF;
                int outU = convertedOutput[uPlaneOffset] & 0xFF;
                int outV = convertedOutput[vPlaneOffset] & 0xFF;
                boolean swapCorrect = (srcU == outU) && (srcV == outV);
                System.out.println("[EncoderDiag] UV swap check: NV21[" + lumaBytes + "]=V("
                        + String.format("%02X", srcV) + ") NV21[" + (lumaBytes + 1) + "]=U("
                        + String.format("%02X", srcU) + ") → out_U=" + String.format("%02X", outU)
                        + " out_V=" + String.format("%02X", outV)
                        + " [" + (swapCorrect ? "OK" : "MISMATCH") + "]");
            }
        } else {
            // NV12 semi-planar: output interleaves UV (swapped from VU).
            StringBuilder uvOut = new StringBuilder("[EncoderDiag] Converted UV interleaved (first 8 bytes @" + lumaBytes + "): ");
            for (int i = lumaBytes; i < Math.min(lumaBytes + 8, convertedOutput.length); i++) {
                uvOut.append(String.format("%02X ", convertedOutput[i] & 0xFF));
            }
            System.out.println(uvOut.toString().trim());

            // UV swap verification for semi-planar.
            if (lumaBytes + 1 < nv21Input.length && lumaBytes + 1 < convertedOutput.length) {
                int srcV = nv21Input[lumaBytes] & 0xFF;
                int srcU = nv21Input[lumaBytes + 1] & 0xFF;
                int outFirst = convertedOutput[lumaBytes] & 0xFF;
                int outSecond = convertedOutput[lumaBytes + 1] & 0xFF;
                boolean swapCorrect = (srcU == outFirst) && (srcV == outSecond);
                System.out.println("[EncoderDiag] UV swap check: NV21[VU]=("
                        + String.format("%02X,%02X", srcV, srcU) + ") → NV12[UV]=("
                        + String.format("%02X,%02X", outFirst, outSecond) + ")"
                        + " [" + (swapCorrect ? "OK" : "MISMATCH") + "]");
            }
        }
        System.out.println("[EncoderDiag] ──────────────────────────────");
    }

    private boolean isFramePayload(MediaCodec.BufferInfo bufferInfo) {
        return (bufferInfo.flags & MediaCodec.BUFFER_FLAG_CODEC_CONFIG) == 0;
    }

    private void requestSyncFrame(String logMessage) {
        try {
            Bundle params = new Bundle();
            params.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0);
            codec.setParameters(params);
            System.out.println(logMessage);
        } catch (RuntimeException e) {
            // Ignore sync-frame request failures; the outer startup probe
            // will still surface the failure if no frame arrives.
        }
    }

    private static int selectInputColorFormat(int[] colorFormats) throws IOException {
        if (colorFormats == null || colorFormats.length == 0) {
            throw new IOException("encoder does not expose any byte-buffer input color formats");
        }

        // Prefer Planar (I420): three independent Y/U/V planes eliminate
        // the NV12-vs-NV21 interleaving ambiguity that affects SemiPlanar
        // on some Android vendor implementations.
        for (int colorFormat : colorFormats) {
            if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar) {
                return colorFormat;
            }
        }

        for (int colorFormat : colorFormats) {
            if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar) {
                return colorFormat;
            }
        }

        for (int colorFormat : colorFormats) {
            if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible) {
                return colorFormat;
            }
        }

        throw new IOException(
                "encoder does not support a usable YUV420 byte-buffer input format: "
                        + describeColorFormats(colorFormats)
        );
    }

    private static String describeColorFormats(int[] colorFormats) {
        StringBuilder builder = new StringBuilder();
        for (int index = 0; index < colorFormats.length; index++) {
            if (index > 0) {
                builder.append(", ");
            }
            builder.append(describeColorFormat(colorFormats[index]));
        }
        return builder.toString();
    }

    private static String describeColorFormat(int colorFormat) {
        if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar) {
            return "COLOR_FormatYUV420SemiPlanar";
        }
        if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar) {
            return "COLOR_FormatYUV420Planar";
        }
        if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible) {
            return "COLOR_FormatYUV420Flexible";
        }
        if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface) {
            return "COLOR_FormatSurface";
        }
        return "0x" + Integer.toHexString(colorFormat);
    }
}
