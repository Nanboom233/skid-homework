package com.skidhomework.server;

import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.net.LocalSocketAddress;

import com.skidhomework.server.camera.ICameraSession;

import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.FilterOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/**
 * One-shot app_process entrypoint that requests a full-resolution still image from
 * the live camera server and either streams the image bytes over stdout or
 * writes them to a device-local file for a later host-side transfer step.
 */
public final class StillCapture {

    private static final String DEFAULT_SOCKET_NAME = "scanner-still";
    private static final byte STATUS_SUCCESS = 0;
    private static final byte STATUS_ERROR = 1;
    private static final int BUFFER_SIZE = 16 * 1024;
    private static final int SOCKET_SEND_BUFFER_BYTES = 4 * 1024 * 1024;
    private static final int OUTPUT_BUFFER_BYTES = 256 * 1024;

    private StillCapture() {
    }

    static final class StatusServer implements AutoCloseable {
        private final String socketName;
        private final AtomicReference<ICameraSession> activeSessionRef;
        private final AtomicBoolean closed = new AtomicBoolean(false);

        private LocalServerSocket serverSocket;
        private Thread acceptThread;

        StatusServer(String socketName, AtomicReference<ICameraSession> activeSessionRef) {
            this.socketName = socketName;
            this.activeSessionRef = activeSessionRef;
        }

        void start() {
            acceptThread = new Thread(this::runAcceptLoop, "StillStatusServer");
            acceptThread.setDaemon(true);
            acceptThread.start();
        }

        @Override
        public void close() {
            if (!closed.compareAndSet(false, true)) {
                return;
            }

            closeQuietly(serverSocket);
            if (acceptThread != null) {
                acceptThread.interrupt();
            }
        }

        private void runAcceptLoop() {
            LocalServerSocket localServerSocket = null;
            try {
                localServerSocket = new LocalServerSocket(socketName);
                serverSocket = localServerSocket;
                System.out.println("[StillCapture] Listening on socket " + socketName + ".");

                while (!closed.get()) {
                    LocalSocket clientSocket = null;
                    try {
                        clientSocket = localServerSocket.accept();
                        handleClient(clientSocket);
                    } catch (IOException e) {
                        if (!closed.get()) {
                            System.err.println("[StillCapture] Accept failed: " + e.getMessage());
                        }
                        closeQuietly(clientSocket);
                    }
                }
            } catch (IOException e) {
                if (!closed.get()) {
                    System.err.println("[StillCapture] Server startup failed: " + e.getMessage());
                }
            } finally {
                closeQuietly(localServerSocket);
                serverSocket = null;
            }
        }

        private void handleClient(LocalSocket clientSocket) {
            try {
                clientSocket.setSendBufferSize(1024 * 1024);
                OutputStream outputStream = clientSocket.getOutputStream();
                try {
                    byte[] imageBytes = requireActiveSession(
                            activeSessionRef,
                            "Camera session is not ready for still capture."
                    ).captureStillJpeg();
                    if (imageBytes == null || imageBytes.length == 0) {
                        throw new IllegalStateException("Still capture returned no image bytes.");
                    }

                    System.out.println("[StillCapture] Socket payload bytes=" + imageBytes.length + ".");

                    outputStream.write(STATUS_SUCCESS);
                    outputStream.write(imageBytes);
                } catch (Exception e) {
                    String message = e.getMessage() == null ? e.toString() : e.getMessage();
                    outputStream.write(STATUS_ERROR);
                    outputStream.write(message.getBytes(StandardCharsets.UTF_8));
                }
                outputStream.flush();
            } catch (IOException e) {
                if (!closed.get()) {
                    System.err.println("[StillCapture] Client handling failed: " + e.getMessage());
                }
            } finally {
                closeQuietly(clientSocket);
            }
        }
    }

    static final class StreamServer implements AutoCloseable {
        private final String socketName;
        private final AtomicReference<ICameraSession> activeSessionRef;
        private final AtomicBoolean closed = new AtomicBoolean(false);

        private LocalServerSocket serverSocket;
        private Thread acceptThread;

        StreamServer(String socketName, AtomicReference<ICameraSession> activeSessionRef) {
            this.socketName = socketName;
            this.activeSessionRef = activeSessionRef;
        }

        void start() {
            acceptThread = new Thread(this::runAcceptLoop, "StillCaptureStreamServer");
            acceptThread.setDaemon(true);
            acceptThread.start();
        }

        @Override
        public void close() {
            if (!closed.compareAndSet(false, true)) {
                return;
            }

            closeQuietly(serverSocket);
            if (acceptThread != null) {
                acceptThread.interrupt();
            }
        }

        private void runAcceptLoop() {
            LocalServerSocket localServerSocket = null;
            try {
                localServerSocket = new LocalServerSocket(socketName);
                serverSocket = localServerSocket;
                System.out.println("[StillStream] Listening on socket " + socketName + ".");

                while (!closed.get()) {
                    LocalSocket clientSocket = null;
                    try {
                        clientSocket = localServerSocket.accept();
                        handoffClient(clientSocket);
                        clientSocket = null;
                    } catch (IOException e) {
                        if (!closed.get()) {
                            System.err.println("[StillStream] Accept failed: " + e.getMessage());
                        }
                        closeQuietly(clientSocket);
                    }
                }
            } catch (IOException e) {
                if (!closed.get()) {
                    System.err.println("[StillStream] Server startup failed: " + e.getMessage());
                }
            } finally {
                closeQuietly(localServerSocket);
                serverSocket = null;
            }
        }

        private void handoffClient(LocalSocket clientSocket) {
            Thread clientThread = new Thread(
                    () -> handleClient(clientSocket),
                    "StillStreamClient-" + System.nanoTime()
            );
            clientThread.setDaemon(true);
            clientThread.start();
        }

        private void handleClient(LocalSocket clientSocket) {
            try {
                clientSocket.setSendBufferSize(SOCKET_SEND_BUFFER_BYTES);
                BufferedOutputStream bufferedOutputStream = new BufferedOutputStream(
                        clientSocket.getOutputStream(),
                        OUTPUT_BUFFER_BYTES
                );
                CountingOutputStream countingOutputStream = new CountingOutputStream(bufferedOutputStream);

                try {
                    requireActiveSession(
                            activeSessionRef,
                            "Camera session is not ready for streamed still capture."
                    ).streamStillJpeg(countingOutputStream);
                    countingOutputStream.flush();
                    System.out.println(
                            "[StillStream] Completed streaming still payload (bytes="
                                    + countingOutputStream.getBytesWritten()
                                    + ")."
                    );
                } catch (Exception e) {
                    String message = e.getMessage() == null ? e.toString() : e.getMessage();
                    if (countingOutputStream.getBytesWritten() == 0) {
                        bufferedOutputStream.write(message.getBytes(StandardCharsets.UTF_8));
                        bufferedOutputStream.flush();
                    }
                    System.err.println("[StillStream] Client handling failed: " + message);
                }
            } catch (IOException e) {
                if (!closed.get()) {
                    System.err.println("[StillStream] Socket I/O failed: " + e.getMessage());
                }
            } finally {
                closeQuietly(clientSocket);
            }
        }
    }

    public static void main(String[] args) {
        Config config = parseArgs(args);

        try {
            byte[] imageBytes = requestStillCapture(config.socketName);
            System.err.println("[StillCapture] Received payload bytes=" + imageBytes.length + ".");
            if (config.outputPath != null && !config.outputPath.isEmpty()) {
                writeStillToFile(config.outputPath, imageBytes);
                System.err.println(
                        "[StillCapture] Wrote still payload to "
                                + config.outputPath
                                + " (bytes="
                                + imageBytes.length
                                + ")"
                );
            } else {
                OutputStream outputStream = System.out;
                outputStream.write(imageBytes);
                outputStream.flush();
            }
        } catch (Throwable throwable) {
            String message = throwable.getMessage() == null ? throwable.toString() : throwable.getMessage();
            System.err.println("[StillCapture] " + message);
            throwable.printStackTrace(System.err);
            System.exit(1);
        }
    }

    private static byte[] requestStillCapture(String socketName) throws IOException {
        LocalSocket socket = new LocalSocket();
        try {
            socket.connect(new LocalSocketAddress(socketName, LocalSocketAddress.Namespace.ABSTRACT));
            InputStream inputStream = socket.getInputStream();
            int status = inputStream.read();
            if (status < 0) {
                throw new IOException("Still capture socket returned no status byte.");
            }

            byte[] payload = readFully(inputStream);
            if (status == STATUS_SUCCESS) {
                if (payload.length == 0) {
                    throw new IOException("Still capture returned an empty payload.");
                }
                return payload;
            }

            String message = new String(payload, StandardCharsets.UTF_8).trim();
            if (message.isEmpty()) {
                message = "Still capture request failed without an error message.";
            }
            if (status == STATUS_ERROR) {
                throw new IOException(message);
            }

            throw new IOException("Still capture returned unknown status " + status + ": " + message);
        } finally {
            try {
                socket.close();
            } catch (IOException e) {
                // Ignore cleanup failures.
            }
        }
    }

    private static ICameraSession requireActiveSession(
            AtomicReference<ICameraSession> activeSessionRef,
            String missingMessage
    ) {
        ICameraSession activeSession = activeSessionRef.get();
        if (activeSession == null) {
            throw new IllegalStateException(missingMessage);
        }
        return activeSession;
    }

    private static void writeStillToFile(String outputPath, byte[] imageBytes) throws IOException {
        File outputFile = new File(outputPath);
        File parent = outputFile.getParentFile();
        if (parent != null && !parent.exists() && !parent.mkdirs() && !parent.isDirectory()) {
            throw new IOException("Failed to create still output directory: " + parent.getAbsolutePath());
        }

        try (FileOutputStream outputStream = new FileOutputStream(outputFile, false)) {
            outputStream.write(imageBytes);
            outputStream.flush();
        }
    }

    private static byte[] readFully(InputStream inputStream) throws IOException {
        ByteArrayOutputStream outputStream = new ByteArrayOutputStream();
        byte[] buffer = new byte[BUFFER_SIZE];
        int read;
        while ((read = inputStream.read(buffer)) != -1) {
            outputStream.write(buffer, 0, read);
        }
        return outputStream.toByteArray();
    }

    private static void closeQuietly(LocalSocket socket) {
        if (socket == null) {
            return;
        }

        try {
            socket.close();
        } catch (IOException e) {
            // Ignore cleanup failures.
        }
    }

    private static void closeQuietly(LocalServerSocket socket) {
        if (socket == null) {
            return;
        }

        try {
            socket.close();
        } catch (IOException e) {
            // Ignore cleanup failures.
        }
    }

    private static Config parseArgs(String[] args) {
        Config config = new Config();

        for (int index = 0; index < args.length; index++) {
            if ("--socket".equals(args[index]) && index + 1 < args.length) {
                config.socketName = args[++index];
            } else if ("--output".equals(args[index]) && index + 1 < args.length) {
                config.outputPath = args[++index];
            } else {
                System.err.println("[StillCapture] Unknown argument: " + args[index]);
            }
        }

        return config;
    }

    private static final class Config {
        String socketName = DEFAULT_SOCKET_NAME;
        String outputPath = null;
    }

    private static final class CountingOutputStream extends FilterOutputStream {
        private long bytesWritten;

        CountingOutputStream(OutputStream delegate) {
            super(delegate);
        }

        @Override
        public void write(int value) throws IOException {
            out.write(value);
            bytesWritten++;
        }

        @Override
        public void write(byte[] buffer, int offset, int length) throws IOException {
            out.write(buffer, offset, length);
            bytesWritten += length;
        }

        long getBytesWritten() {
            return bytesWritten;
        }
    }
}
