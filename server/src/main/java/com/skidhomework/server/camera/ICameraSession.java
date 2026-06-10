package com.skidhomework.server.camera;

import java.io.OutputStream;

public interface ICameraSession {
    void start() throws Exception;

    void stop();

    byte[] captureStillJpeg() throws Exception;

    void streamStillJpeg(OutputStream outputStream) throws Exception;
}
