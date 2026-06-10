package com.skidhomework.server;

import java.io.BufferedOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;

/**
 * Writes length-prefixed H.264 NAL units to a socket output stream.
 *
 * <p>Each NAL unit is framed as:
 * <pre>
 *   [4 bytes big-endian length] [NAL unit data]
 * </pre>
 *
 * <p>This matches the protocol expected by the Rust stream decoder
 * ({@code stream_decoder.rs}).
 *
 * <p>Performance optimisations for 1080p30 throughput:
 * <ul>
 *   <li>Wraps the raw socket stream in a 32 KB {@link BufferedOutputStream}
 *       so multiple small NAL units (SPS/PPS + slice) coalesce into fewer
 *       underlying socket writes through the ADB tunnel.</li>
 *   <li>Merges the 4-byte length header and the payload into a single
 *       {@code write()} call via a reusable staging buffer, halving the
 *       per-frame system call count.</li>
 *   <li>Accepts {@link ByteBuffer} directly from MediaCodec output buffers
 *       to eliminate the intermediate {@code byte[]} copy.</li>
 * </ul>
 */
public final class SocketRelay {

    private static final int BUFFERED_STREAM_SIZE = 32 * 1024;
    /**
     * Staging buffer capacity.  Sized for 1080p IDR frames which can reach
     * ~120 KB.  Automatically grown if a larger NAL is encountered.
     */
    private static final int INITIAL_STAGING_CAPACITY = 128 * 1024;

    private final BufferedOutputStream outputStream;
    private final Consumer<StopReason> stopCallback;
    private final AtomicBoolean closed = new AtomicBoolean(false);

    /** Reusable staging buffer: [4-byte header | payload].  Grown on demand. */
    private byte[] stagingBuffer;

    public SocketRelay(OutputStream rawOutputStream, Consumer<StopReason> stopCallback) {
        this.outputStream = new BufferedOutputStream(rawOutputStream, BUFFERED_STREAM_SIZE);
        this.stopCallback = stopCallback;
        this.stagingBuffer = new byte[INITIAL_STAGING_CAPACITY];
    }

    /**
     * Send a single NAL unit with a 4-byte big-endian length prefix.
     *
     * <p>The header and payload are merged into a single {@code write()} call
     * to halve the per-frame system call overhead.
     *
     * @param nalData the raw H.264 NAL unit bytes
     * @throws IOException if writing to the socket fails
     */
    public synchronized void sendNalUnit(byte[] nalData) throws IOException {
        sendNalUnit(nalData, 0, nalData.length);
    }

    /**
     * Send a NAL unit from a sub-range of a byte array.
     */
    public synchronized void sendNalUnit(byte[] nalData, int offset, int length) throws IOException {
        if (closed.get()) {
            throw new IOException("socket relay is closed");
        }

        int totalLength = 4 + length;
        byte[] buffer = ensureStagingCapacity(totalLength);

        // Write 4-byte big-endian length header.
        buffer[0] = (byte) ((length >> 24) & 0xFF);
        buffer[1] = (byte) ((length >> 16) & 0xFF);
        buffer[2] = (byte) ((length >> 8) & 0xFF);
        buffer[3] = (byte) (length & 0xFF);

        // Copy payload immediately after header.
        System.arraycopy(nalData, offset, buffer, 4, length);

        writeToSocket(buffer, totalLength);
    }

    /**
     * Send a NAL unit directly from a MediaCodec output {@link ByteBuffer},
     * eliminating the intermediate {@code byte[]} allocation that the legacy
     * {@code byte[]} overload requires.
     *
     * @param nalBuffer the MediaCodec output ByteBuffer
     * @param offset    start position of the NAL data within the buffer
     * @param length    number of bytes to send
     * @throws IOException if writing to the socket fails
     */
    public synchronized void sendNalUnit(ByteBuffer nalBuffer, int offset, int length)
            throws IOException {
        if (closed.get()) {
            throw new IOException("socket relay is closed");
        }

        int totalLength = 4 + length;
        byte[] buffer = ensureStagingCapacity(totalLength);

        // Write 4-byte big-endian length header.
        buffer[0] = (byte) ((length >> 24) & 0xFF);
        buffer[1] = (byte) ((length >> 16) & 0xFF);
        buffer[2] = (byte) ((length >> 8) & 0xFF);
        buffer[3] = (byte) (length & 0xFF);

        // Bulk-get payload from the ByteBuffer directly into the staging buffer.
        nalBuffer.position(offset);
        nalBuffer.get(buffer, 4, length);

        writeToSocket(buffer, totalLength);
    }

    /**
     * Close the underlying socket stream.
     */
    public synchronized void close() {
        if (!closed.compareAndSet(false, true)) {
            return;
        }

        try {
            outputStream.flush();
        } catch (IOException e) {
            // Best-effort flush before close.
        }
        try {
            outputStream.close();
        } catch (IOException e) {
            // Ignore cleanup failures.
        }
    }

    // --- Internal helpers ---

    private byte[] ensureStagingCapacity(int required) {
        if (stagingBuffer.length < required) {
            // Grow to next power-of-two that satisfies the requirement.
            int newCapacity = Integer.highestOneBit(required - 1) << 1;
            stagingBuffer = new byte[Math.max(newCapacity, required)];
        }
        return stagingBuffer;
    }

    private void writeToSocket(byte[] buffer, int length) throws IOException {
        try {
            outputStream.write(buffer, 0, length);
        } catch (IOException e) {
            System.err.println("[Socket] Failed to write frame to upstream client: " + e.getMessage());
            stopCallback.accept(StopReason.socketWriteFailed("socket write failed: " + e.getMessage()));
            throw e;
        }
    }
}
