# Linux runtime placeholder

Expected runtime files for the planned native desktop backend:

- `onnxruntime/linux/libonnxruntime.so`
- `onnxruntime/linux/libonnxruntime_providers_tensorrt.so`
- `onnxruntime/linux/libonnxruntime_providers_cuda.so`

Official upstream release artifact currently targeted:

- `onnxruntime-linux-x64-gpu-1.24.4.tgz`

The Linux path remains unstaged in this Windows workspace, but the runtime contract
has been kept explicit so later TensorRT / CUDA packaging does not drift.
