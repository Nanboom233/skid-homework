package com.skidhomework.server.encoder;

import java.io.IOException;

public interface IEncoder {
    void start();

    void awaitFirstFrame(long timeoutMs) throws InterruptedException, IOException;

    void stop();
}
