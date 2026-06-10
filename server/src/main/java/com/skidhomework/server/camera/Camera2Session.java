package com.skidhomework.server.camera;

import android.annotation.SuppressLint;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Rect;
import android.hardware.camera2.CameraAccessException;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.CaptureResult;
import android.hardware.camera2.CaptureFailure;
import android.hardware.camera2.TotalCaptureResult;
import android.graphics.ImageFormat;
import android.media.Image;
import android.media.ImageReader;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.SystemClock;
import android.util.Size;
import android.view.Surface;

import com.skidhomework.server.StopReason;

import java.io.ByteArrayOutputStream;
import java.io.FilterOutputStream;
import java.io.FileReader;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.Collections;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

/**
 * Opens an Android camera via Camera2 API and sends frames to an encoder Surface.
 *
 * <p>Since this runs via {@code app_process} at the shell UID level,
 * camera permissions are bypassed (same approach as scrcpy).
 */
public final class Camera2Session implements ICameraSession {
    private static final int CAMERA_OPEN_TIMEOUT_SECONDS = 5;
    private static final int MAX_INTERNAL_CAMERA_RECOVERY_ATTEMPTS = 4;
    private static final long CAMERA_RECOVERY_DELAY_BASE_MS = 150L;
    private static final long CAMERA_RECOVERY_DELAY_MAX_MS = 5_000L;
    private static final long FIRST_CAPTURE_START_TIMEOUT_MS = 1_500L;
    private static final long STILL_CAPTURE_TIMEOUT_MS = 4_000L;
    private static final byte JPEG_QUALITY = (byte) 95;
    private static final int NORMALIZED_STILL_JPEG_QUALITY = 100;
    private static final String THERMAL_ZONE_TYPE_PATH = "/sys/class/thermal/thermal_zone0/type";

    private final String cameraId;
    private final int width;
    private final int height;
    private final Surface encoderSurface;
    private final Consumer<StopReason> stopCallback;
    private final boolean deferStillSurfaceUntilCapture;
    private final HandlerThread handlerThread;
    private final Handler handler;
    private final AtomicBoolean stopping = new AtomicBoolean(false);
    private final AtomicBoolean disconnectReported = new AtomicBoolean(false);
    private final AtomicBoolean recoveryInProgress = new AtomicBoolean(false);
    private final AtomicBoolean stopReported = new AtomicBoolean(false);
    private final AtomicBoolean startCompleted = new AtomicBoolean(false);
    private final Object cameraLock = new Object();
    private final Object stillCaptureLock = new Object();

    private CameraDevice cameraDevice;
    private CameraCaptureSession captureSession;
    private CameraManager cameraManager;
    private CameraCharacteristics cameraCharacteristics;
    private ImageReader stillImageReader;
    private Size stillCaptureSize;
    private int stillJpegOrientation;
    private Float previewFocusDistance;

    public Camera2Session(
            String cameraId,
            int width,
            int height,
            Surface encoderSurface,
            boolean deferStillSurfaceUntilCapture,
            Consumer<StopReason> stopCallback
    ) {
        this.cameraId = cameraId;
        this.width = width;
        this.height = height;
        this.encoderSurface = encoderSurface;
        this.deferStillSurfaceUntilCapture = deferStillSurfaceUntilCapture;
        this.stopCallback = stopCallback;

        handlerThread = new HandlerThread("CameraThread");
        handlerThread.start();
        handler = new Handler(handlerThread.getLooper());
    }

    /**
     * Open the camera and start a repeating capture request targeting the encoder surface.
     */
    @SuppressLint("MissingPermission") // Permissions bypassed at shell UID level
    public void start() throws Exception {
        ensureCameraManager();
        openCameraPipeline();
        startCompleted.set(true);
        disconnectReported.set(false);
    }

    /**
     * Capture a single high-resolution JPEG from the active camera session.
     */
    public byte[] captureStillJpeg() throws Exception {
        byte[] imageBytes = captureRawStillJpeg();
        return normalizeStillJpegForBrowserCompatibility(imageBytes);
    }

    /**
     * Capture and stream a browser-compatible high-resolution JPEG directly to the supplied output.
     */
    public void streamStillJpeg(OutputStream outputStream) throws Exception {
        byte[] imageBytes = captureRawStillJpeg();
        writeNormalizedStillJpegForBrowserCompatibility(imageBytes, outputStream);
    }

    private byte[] captureRawStillJpeg() throws Exception {
        synchronized (stillCaptureLock) {
            if (stopping.get()) {
                throw new IllegalStateException("Camera pipeline is stopping.");
            }

            final CameraDevice activeCamera;
            final CameraCaptureSession activeSession;
            final ImageReader activeReader;
            synchronized (cameraLock) {
                activeCamera = cameraDevice;
                activeSession = captureSession;
                activeReader = stillImageReader;
            }

            if (activeCamera == null || activeSession == null || activeReader == null) {
                throw new IllegalStateException("Camera still capture is not ready.");
            }

            if (deferStillSurfaceUntilCapture) {
                throw new IllegalStateException(
                        "Still capture is disabled while startup still surface deferral is enabled."
                );
            }

            clearPendingStillImage(activeReader);

            final CountDownLatch imageLatch = new CountDownLatch(1);
            final AtomicReference<byte[]> imageBytesRef = new AtomicReference<>();
            final AtomicReference<String> failureMessageRef = new AtomicReference<>();

            activeReader.setOnImageAvailableListener(
                    (reader) -> {
                        Image image = null;
                        try {
                            image = reader.acquireLatestImage();
                            if (image == null) {
                                return;
                            }

                            ByteBuffer buffer = image.getPlanes()[0].getBuffer();
                            byte[] bytes = new byte[buffer.remaining()];
                            buffer.get(bytes);
                            imageBytesRef.compareAndSet(null, bytes);
                            imageLatch.countDown();
                        } catch (RuntimeException e) {
                            failureMessageRef.compareAndSet(
                                    null,
                                    "Failed to read still image: " + sanitizeExceptionMessage(e)
                            );
                            imageLatch.countDown();
                        } finally {
                            if (image != null) {
                                image.close();
                            }
                        }
                    },
                    handler
            );

            CaptureRequest.Builder requestBuilder =
                    activeCamera.createCaptureRequest(CameraDevice.TEMPLATE_VIDEO_SNAPSHOT);
            requestBuilder.addTarget(activeReader.getSurface());
            requestBuilder.addTarget(encoderSurface);
            requestBuilder.set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO);
            requestBuilder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON);
            requestBuilder.set(
                    CaptureRequest.CONTROL_AF_MODE,
                    CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO
            );
            Float lockedPreviewFocusDistance = getClampedPreviewFocusDistance();
            if (lockedPreviewFocusDistance != null) {
                requestBuilder.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_OFF);
                requestBuilder.set(CaptureRequest.LENS_FOCUS_DISTANCE, lockedPreviewFocusDistance);
            }
            requestBuilder.set(CaptureRequest.JPEG_QUALITY, JPEG_QUALITY);
            requestBuilder.set(CaptureRequest.JPEG_ORIENTATION, stillJpegOrientation);
            final CaptureRequest stillRequest = requestBuilder.build();

            System.out.println(
                    "[StillCapture] Submitting preview-aligned still request (surfaces=still+preview): "
                            + describeCaptureRequest(stillRequest)
                            + " previewFocusLock="
                            + (lockedPreviewFocusDistance == null ? "none" : lockedPreviewFocusDistance)
            );

            try {
                activeSession.capture(
                        stillRequest,
                        new CameraCaptureSession.CaptureCallback() {
                            @Override
                            public void onCaptureFailed(
                                    CameraCaptureSession session,
                                    CaptureRequest request,
                                    CaptureFailure failure
                            ) {
                                failureMessageRef.compareAndSet(
                                        null,
                                        "Still capture request failed: reason=" + failure.getReason()
                                );
                                imageLatch.countDown();
                            }

                            @Override
                            public void onCaptureCompleted(
                                    CameraCaptureSession session,
                                    CaptureRequest request,
                                    TotalCaptureResult result
                            ) {
                                System.out.println(
                                        "[StillCapture] Capture result: "
                                                + describeCaptureResult(result)
                                );
                            }
                        },
                        handler
                );
            } catch (CameraAccessException | IllegalStateException e) {
                activeReader.setOnImageAvailableListener(null, null);
                throw new RuntimeException(
                        "failed to submit still capture request: " + sanitizeExceptionMessage(e),
                        e
                );
            }

            boolean imageArrived = imageLatch.await(STILL_CAPTURE_TIMEOUT_MS, TimeUnit.MILLISECONDS);
            activeReader.setOnImageAvailableListener(null, null);

            if (!imageArrived) {
                throw new RuntimeException(
                        "still capture timed out after " + STILL_CAPTURE_TIMEOUT_MS + "ms"
                );
            }

            String failureMessage = failureMessageRef.get();
            if (failureMessage != null) {
                throw new RuntimeException(failureMessage);
            }

            byte[] imageBytes = imageBytesRef.get();
            if (imageBytes == null || imageBytes.length == 0) {
                throw new RuntimeException("still capture returned no image bytes");
            }

            System.out.println("[StillCapture] Camera still JPEG bytes=" + imageBytes.length + ".");

            return imageBytes;
        }
    }

    private void ensureCameraManager() throws Exception {
        if (cameraManager != null && cameraCharacteristics != null) {
            return;
        }

        try {
            cameraManager = CameraSupport.createShellCameraManager();
        } catch (Exception e) {
            throw new RuntimeException("Failed to obtain CameraManager system service.", e);
        }

        if (cameraManager == null) {
            throw new RuntimeException("Failed to obtain CameraManager system service.");
        }

        cameraCharacteristics = CameraSupport.getCameraCharacteristics(cameraManager, cameraId);
    }

    @SuppressLint("MissingPermission") // Permissions bypassed at shell UID level
    private void openCameraPipeline() throws Exception {
        closeCurrentPipeline(null);
        long cameraOpenStartedAtMs = SystemClock.elapsedRealtime();
        CountDownLatch openLatch = new CountDownLatch(1);

        cameraManager.openCamera(cameraId, new CameraDevice.StateCallback() {
            @Override
            public void onOpened(CameraDevice camera) {
                if (stopping.get()) {
                    camera.close();
                    openLatch.countDown();
                    return;
                }
                synchronized (cameraLock) {
                    cameraDevice = camera;
                }
                openLatch.countDown();
            }

            @Override
            public void onDisconnected(CameraDevice camera) {
                handleRecoverableCameraIssue(
                        camera,
                        StopReason.cameraDisconnected(),
                        "[Camera] Disconnected unexpectedly. Attempting in-process camera recovery."
                );
                openLatch.countDown();
            }

            @Override
            public void onError(CameraDevice camera, int error) {
                handleRecoverableCameraIssue(
                        camera,
                        StopReason.cameraError(error),
                        "[Camera] Error "
                                + error
                                + ". Attempting in-process camera recovery."
                );
                openLatch.countDown();
            }
        }, handler);

        if (!openLatch.await(CAMERA_OPEN_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            throw new RuntimeException(
                    "camera open timed out after " + CAMERA_OPEN_TIMEOUT_SECONDS + "s"
            );
        }

        if (cameraDevice == null) {
            throw new RuntimeException("failed to open camera " + cameraId);
        }

        double previewAspect = CameraSupport.normalizedAspectRatio(width, height);
        stillCaptureSize = CameraSupport.selectOutputSizeForAspect(
                cameraCharacteristics,
                ImageFormat.JPEG,
                previewAspect,
                Long.MAX_VALUE
        );
        stillImageReader = ImageReader.newInstance(
                stillCaptureSize.getWidth(),
                stillCaptureSize.getHeight(),
                ImageFormat.JPEG,
                2
        );
        stillJpegOrientation = resolveStillJpegOrientation();

        System.out.println(
                "[Camera] Opened camera "
                        + cameraId
                        + " in "
                        + (SystemClock.elapsedRealtime() - cameraOpenStartedAtMs)
                        + "ms."
        );
        System.out.println(
                "[Camera] Still capture output size: "
                        + stillCaptureSize.getWidth()
                        + "x"
                        + stillCaptureSize.getHeight()
                        + "."
        );
        System.out.println(
                "[Camera] Still JPEG orientation: "
                        + stillJpegOrientation
                        + " degrees."
        );

        // Create a shared session so preview streaming and still capture use the same camera pipeline.
        // For diagnostics on devices where shell-only session startup regressed after the shared
        // still surface was introduced, allow startup to bind only the preview encoder surface.
        long sessionConfigureStartedAtMs = SystemClock.elapsedRealtime();
        CountDownLatch sessionLatch = new CountDownLatch(1);
        java.util.List<Surface> startupSessionSurfaces = deferStillSurfaceUntilCapture
                ? Collections.singletonList(encoderSurface)
                : Arrays.asList(encoderSurface, stillImageReader.getSurface());

        System.out.println(
                "[Camera] Startup session mode: "
                        + (deferStillSurfaceUntilCapture ? "preview-only" : "preview+still")
                        + "."
        );

        try {
            cameraDevice.createCaptureSession(
                    startupSessionSurfaces,
                    new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession session) {
                            if (stopping.get()) {
                                try {
                                    session.close();
                                } catch (RuntimeException e) {
                                    // Ignore session close failures during shutdown.
                                }
                                sessionLatch.countDown();
                                return;
                            }
                            synchronized (cameraLock) {
                                captureSession = session;
                            }
                            sessionLatch.countDown();
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession session) {
                            System.err.println("[Camera] Session configuration failed.");
                            if (shouldUseInProcessRecovery()) {
                                handleRecoverableSessionIssue(
                                        session,
                                        StopReason.cameraSessionFailed("camera session configuration failed"),
                                        "[Camera] Session configuration failed. Attempting in-process camera recovery."
                                );
                            } else {
                                closeCaptureSession(session);
                            }
                            sessionLatch.countDown();
                        }
                    },
                    handler
            );
        } catch (RuntimeException e) {
            throw new RuntimeException(
                    "failed to invoke createCaptureSession: " + sanitizeExceptionMessage(e),
                    e
            );
        }

        if (!sessionLatch.await(CAMERA_OPEN_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            throw new RuntimeException("camera session creation timed out");
        }

        if (captureSession == null) {
            String thermalProbeIssue = detectThermalProbeReadabilityIssue();
            if (thermalProbeIssue != null) {
                throw new RuntimeException(
                        "device thermal probe is not readable: "
                                + THERMAL_ZONE_TYPE_PATH
                                + " ("
                                + thermalProbeIssue
                                + ")"
                );
            }
            throw new RuntimeException("failed to configure camera capture session");
        }

        System.out.println(
                "[Camera] Capture session configured in "
                        + (SystemClock.elapsedRealtime() - sessionConfigureStartedAtMs)
                        + "ms."
        );

        // Build a repeating capture request for continuous streaming
        CaptureRequest.Builder requestBuilder =
                cameraDevice.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
        requestBuilder.addTarget(encoderSurface);
        requestBuilder.set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO);
        requestBuilder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON);
        requestBuilder.set(CaptureRequest.CONTROL_AF_MODE,
                CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO);

        final CaptureRequest repeatingRequest = requestBuilder.build();
        System.out.println(
                "[Camera] Preview repeating request (surfaces=preview): "
                        + describeCaptureRequest(repeatingRequest)
        );

        final long repeatingRequestStartedAtMs = SystemClock.elapsedRealtime();
        final CountDownLatch firstCaptureLatch = new CountDownLatch(1);
        final AtomicBoolean firstCaptureReported = new AtomicBoolean(false);
        final AtomicBoolean firstCaptureResultReported = new AtomicBoolean(false);

        try {
            captureSession.setRepeatingRequest(
                    repeatingRequest,
                    new CameraCaptureSession.CaptureCallback() {
                        @Override
                        public void onCaptureStarted(
                                CameraCaptureSession session,
                                CaptureRequest request,
                                long timestamp,
                                long frameNumber
                        ) {
                            if (firstCaptureReported.compareAndSet(false, true)) {
                                long firstCaptureLatencyMs =
                                        SystemClock.elapsedRealtime() - repeatingRequestStartedAtMs;
                                System.out.println(
                                        "[Camera] First capture started in "
                                                + firstCaptureLatencyMs
                                                + "ms after repeating request."
                                );
                                firstCaptureLatch.countDown();
                            }
                        }

                        @Override
                        public void onCaptureCompleted(
                                CameraCaptureSession session,
                                CaptureRequest request,
                                TotalCaptureResult result
                        ) {
                            updatePreviewFocusDistance(result);
                            if (firstCaptureResultReported.compareAndSet(false, true)) {
                                System.out.println(
                                        "[Camera] Preview first capture result: "
                                                + describeCaptureResult(result)
                                );
                            }
                        }
                    },
                    handler
            );
        } catch (CameraAccessException | IllegalStateException e) {
            closeCurrentPipeline(null);
            throw new RuntimeException("failed to start repeating request: " + e.getMessage(), e);
        }

        if (!firstCaptureLatch.await(FIRST_CAPTURE_START_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
            closeCurrentPipeline(null);
            throw new RuntimeException(
                    "camera first capture timed out after "
                            + FIRST_CAPTURE_START_TIMEOUT_MS
                            + "ms"
            );
        }

        System.out.println("[Camera] Capture session started.");
    }

    /**
     * Stop capture and release camera resources.
     */
    public void stop() {
        if (!stopping.compareAndSet(false, true)) {
            return;
        }

        startCompleted.set(false);
        closeCurrentPipeline(null);
        handlerThread.quitSafely();
        try {
            handlerThread.join(1_000);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private void handleRecoverableCameraIssue(
            CameraDevice camera,
            StopReason reason,
            String logMessage
    ) {
        boolean expectedShutdown = stopping.get();
        closeCurrentPipeline(camera);
        if (expectedShutdown) {
            System.out.println("[Camera] Camera callback received during shutdown.");
            return;
        }

        if (!startCompleted.get()) {
            return;
        }

        scheduleInProcessRecovery(reason, logMessage);
    }

    private void handleRecoverableSessionIssue(
            CameraCaptureSession session,
            StopReason reason,
            String logMessage
    ) {
        closeCaptureSession(session);
        if (stopping.get() || !startCompleted.get()) {
            return;
        }

        scheduleInProcessRecovery(reason, logMessage);
    }

    private boolean shouldUseInProcessRecovery() {
        return !stopping.get() && startCompleted.get() && !recoveryInProgress.get();
    }

    private void scheduleInProcessRecovery(StopReason initialReason, String logMessage) {
        if (!recoveryInProgress.compareAndSet(false, true)) {
            return;
        }

        if (disconnectReported.compareAndSet(false, true)) {
            System.err.println(logMessage);
        }

        Thread recoveryThread = new Thread(
                () -> runInProcessRecovery(initialReason),
                "CameraRecovery"
        );
        recoveryThread.setDaemon(true);
        recoveryThread.start();
    }

    private void runInProcessRecovery(StopReason initialReason) {
        StopReason lastReason = initialReason;

        try {
            for (int attempt = 1; attempt <= MAX_INTERNAL_CAMERA_RECOVERY_ATTEMPTS; attempt++) {
                if (stopping.get()) {
                    return;
                }

                long delayMs = computeRecoveryDelayMs(attempt);
                if (delayMs > 0L) {
                    SystemClock.sleep(delayMs);
                }
                if (stopping.get()) {
                    return;
                }

                System.out.println(
                        "[Camera] Recovery attempt #"
                                + attempt
                                + " after "
                                + lastReason
                                + "."
                );

                try {
                    openCameraPipeline();
                    disconnectReported.set(false);
                    System.out.println(
                            "[Camera] Camera pipeline recovered on attempt #"
                                    + attempt
                                    + "."
                    );
                    return;
                } catch (Exception e) {
                    lastReason = StopReason.cameraStartFailed(
                            "camera recovery attempt #"
                                    + attempt
                                    + " failed: "
                                    + sanitizeExceptionMessage(e)
                    );
                    System.err.println("[Camera] " + lastReason.getMessage());
                }
            }
        } finally {
            recoveryInProgress.set(false);
        }

        requestStop(
                StopReason.cameraStartFailed(
                        "camera recovery exhausted after "
                                + MAX_INTERNAL_CAMERA_RECOVERY_ATTEMPTS
                                + " attempts: "
                                + lastReason.getMessage()
                )
        );
    }

    private long computeRecoveryDelayMs(int attempt) {
        long delayMs = CAMERA_RECOVERY_DELAY_BASE_MS;
        for (int index = 1; index < attempt; index++) {
            delayMs = Math.min(delayMs * 2L, CAMERA_RECOVERY_DELAY_MAX_MS);
        }
        return delayMs;
    }

    private void requestStop(StopReason reason) {
        if (stopping.get()) {
            return;
        }
        if (stopReported.compareAndSet(false, true)) {
            stopCallback.accept(reason);
        }
    }

    private String sanitizeExceptionMessage(Throwable throwable) {
        if (throwable == null) {
            return "unknown error";
        }

        Throwable cause = throwable.getCause() != null ? throwable.getCause() : throwable;
        String message = cause.getMessage();
        if (message != null && !message.trim().isEmpty()) {
            return message.trim();
        }
        return cause.toString();
    }

    private String detectThermalProbeReadabilityIssue() {
        try (FileReader reader = new FileReader(THERMAL_ZONE_TYPE_PATH)) {
            reader.read();
            return null;
        } catch (IOException e) {
            return sanitizeExceptionMessage(e);
        }
    }

    private void closeCurrentPipeline(CameraDevice callbackCamera) {
        synchronized (cameraLock) {
            closeCaptureSessionLocked();
            closeStillImageReaderLocked();
            previewFocusDistance = null;
            if (callbackCamera != null) {
                closeCameraDeviceLocked(callbackCamera);
            } else if (cameraDevice != null) {
                closeCameraDeviceLocked(cameraDevice);
            }
        }
    }

    private void closeCaptureSession(CameraCaptureSession session) {
        try {
            session.close();
        } catch (RuntimeException e) {
            // Ignore close failures during teardown and recovery.
        }
        synchronized (cameraLock) {
            if (captureSession == session) {
                captureSession = null;
            }
        }
    }

    private void closeCaptureSessionLocked() {
        if (captureSession == null) {
            return;
        }

        try {
            captureSession.stopRepeating();
        } catch (CameraAccessException | RuntimeException e) {
            // Ignore errors during cleanup.
        }
        try {
            captureSession.abortCaptures();
        } catch (CameraAccessException | RuntimeException e) {
            // Ignore errors during cleanup.
        }
        try {
            captureSession.close();
        } catch (RuntimeException e) {
            // Ignore errors during cleanup.
        }
        captureSession = null;
    }

    private void closeStillImageReaderLocked() {
        if (stillImageReader == null) {
            return;
        }

        try {
            stillImageReader.setOnImageAvailableListener(null, null);
            stillImageReader.close();
        } catch (RuntimeException e) {
            // Ignore cleanup failures during teardown and recovery.
        }
        stillImageReader = null;
        stillCaptureSize = null;
    }

    private int resolveStillJpegOrientation() {
        if (cameraCharacteristics == null || stillCaptureSize == null) {
            return 0;
        }

        boolean referenceLandscape = width >= height;
        boolean stillLandscape = stillCaptureSize.getWidth() >= stillCaptureSize.getHeight();
        if (referenceLandscape == stillLandscape) {
            return 0;
        }

        Integer sensorOrientationValue = cameraCharacteristics.get(
                CameraCharacteristics.SENSOR_ORIENTATION
        );
        if (sensorOrientationValue == null) {
            return 90;
        }

        int sensorOrientationDegrees = ((sensorOrientationValue % 360) + 360) % 360;
        if (sensorOrientationDegrees == 90 || sensorOrientationDegrees == 270) {
            return sensorOrientationDegrees;
        }

        return 90;
    }

    private void closeCameraDeviceLocked(CameraDevice camera) {
        try {
            camera.close();
        } catch (RuntimeException e) {
            // Ignore camera close failures during teardown and recovery.
        }
        if (cameraDevice == camera) {
            cameraDevice = null;
        }
    }

    private void clearPendingStillImage(ImageReader reader) {
        Image image = null;
        try {
            image = reader.acquireLatestImage();
            while (image != null) {
                image.close();
                image = reader.acquireLatestImage();
            }
        } catch (RuntimeException e) {
            if (image != null) {
                image.close();
            }
        }
    }

    private void updatePreviewFocusDistance(TotalCaptureResult result) {
        Float focusDistance = result.get(CaptureResult.LENS_FOCUS_DISTANCE);
        synchronized (cameraLock) {
            previewFocusDistance = focusDistance;
        }
    }

    private Float getClampedPreviewFocusDistance() {
        synchronized (cameraLock) {
            if (cameraCharacteristics == null || previewFocusDistance == null) {
                return null;
            }

            Float minimumFocusDistance = cameraCharacteristics.get(
                    CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE
            );
            if (minimumFocusDistance == null || !Float.isFinite(previewFocusDistance)) {
                return null;
            }

            return Math.max(0f, Math.min(previewFocusDistance, minimumFocusDistance));
        }
    }

    private static String describeCaptureRequest(CaptureRequest request) {
        return "intent="
                + request.get(CaptureRequest.CONTROL_CAPTURE_INTENT)
                + " aeMode="
                + request.get(CaptureRequest.CONTROL_AE_MODE)
                + " afMode="
                + request.get(CaptureRequest.CONTROL_AF_MODE)
                + " crop="
                + describeRect(request.get(CaptureRequest.SCALER_CROP_REGION))
                + " zoom="
                + describeRequestZoomRatio(request)
                + " videoStab="
                + request.get(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE)
                + " opticalStab="
                + request.get(CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE)
                + " focusDistance="
                + request.get(CaptureRequest.LENS_FOCUS_DISTANCE)
                + " jpegOrientation="
                + request.get(CaptureRequest.JPEG_ORIENTATION);
    }

    private static String describeCaptureResult(TotalCaptureResult result) {
        return "intent="
                + result.get(CaptureResult.CONTROL_CAPTURE_INTENT)
                + " aeMode="
                + result.get(CaptureResult.CONTROL_AE_MODE)
                + " afMode="
                + result.get(CaptureResult.CONTROL_AF_MODE)
                + " crop="
                + describeRect(result.get(CaptureResult.SCALER_CROP_REGION))
                + " zoom="
                + describeResultZoomRatio(result)
                + " videoStab="
                + result.get(CaptureResult.CONTROL_VIDEO_STABILIZATION_MODE)
                + " opticalStab="
                + result.get(CaptureResult.LENS_OPTICAL_STABILIZATION_MODE)
                + " focalLength="
                + result.get(CaptureResult.LENS_FOCAL_LENGTH)
                + " focusDistance="
                + result.get(CaptureResult.LENS_FOCUS_DISTANCE)
                + " jpegOrientation="
                + result.get(CaptureResult.JPEG_ORIENTATION);
    }

    private static String describeRequestZoomRatio(CaptureRequest request) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return "n/a";
        }

        Float zoomRatio = request.get(CaptureRequest.CONTROL_ZOOM_RATIO);
        return zoomRatio == null ? "default" : zoomRatio.toString();
    }

    private static String describeResultZoomRatio(TotalCaptureResult result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return "n/a";
        }

        Float zoomRatio = result.get(CaptureResult.CONTROL_ZOOM_RATIO);
        return zoomRatio == null ? "default" : zoomRatio.toString();
    }

    private static String describeRect(Rect rect) {
        if (rect == null) {
            return "default";
        }
        return rect.left + "," + rect.top + "-" + rect.right + "," + rect.bottom;
    }

    private static byte[] normalizeStillJpegForBrowserCompatibility(byte[] imageBytes) {
        ByteArrayOutputStream outputStream = new ByteArrayOutputStream(
                Math.max(imageBytes.length, 64 * 1024)
        );
        final long normalizedLength;
        try {
            normalizedLength = writeNormalizedStillJpegForBrowserCompatibility(imageBytes, outputStream);
        } catch (IOException error) {
            throw new RuntimeException(
                    "I/O failure while normalizing still JPEG for browser compatibility.",
                    error
            );
        }
        byte[] normalizedBytes = outputStream.toByteArray();
        if (normalizedBytes.length == 0 || normalizedLength == 0) {
            throw new RuntimeException("Normalized still JPEG is empty.");
        }
        return normalizedBytes;
    }

    private static long writeNormalizedStillJpegForBrowserCompatibility(
            byte[] imageBytes,
            OutputStream outputStream
    ) throws IOException {
        Bitmap bitmap = null;
        try {
            BitmapFactory.Options decodeOptions = new BitmapFactory.Options();
            decodeOptions.inPreferredConfig = Bitmap.Config.ARGB_8888;
            decodeOptions.inScaled = false;

            bitmap = BitmapFactory.decodeByteArray(
                    imageBytes,
                    0,
                    imageBytes.length,
                    decodeOptions
            );
            if (bitmap == null) {
                throw new RuntimeException(
                        "BitmapFactory could not decode still JPEG payload for browser normalization."
                );
            }

            CountingOutputStream countingOutputStream = new CountingOutputStream(outputStream);
            boolean compressed = bitmap.compress(
                    Bitmap.CompressFormat.JPEG,
                    NORMALIZED_STILL_JPEG_QUALITY,
                    countingOutputStream
            );
            if (!compressed) {
                throw new RuntimeException(
                        "Bitmap.compress returned false while normalizing still JPEG."
                );
            }

            countingOutputStream.flush();
            long normalizedLength = countingOutputStream.getBytesWritten();
            if (normalizedLength == 0) {
                throw new RuntimeException("Normalized still JPEG is empty.");
            }

            System.out.println(
                    "[StillCapture] Browser-compatible still JPEG summary: "
                            + " rawLen="
                            + imageBytes.length
                            + " normalizedLen="
                            + normalizedLength
                            + " bitmap="
                            + bitmap.getWidth()
                            + "x"
                            + bitmap.getHeight()
                            + "."
            );

            return normalizedLength;
        } catch (OutOfMemoryError error) {
            System.err.println(
                    "[StillCapture] Browser-compatible JPEG normalization ran out of memory. rawLen="
                            + imageBytes.length
            );
            throw new RuntimeException("Out of memory while normalizing still JPEG.", error);
        } catch (RuntimeException error) {
            String message = error.getMessage() == null ? error.toString() : error.getMessage();
            System.err.println(
                    "[StillCapture] Browser-compatible JPEG normalization failed: "
                            + message
                            + " rawLen="
                            + imageBytes.length
            );
            throw error;
        } finally {
            if (bitmap != null) {
                bitmap.recycle();
            }
        }
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
