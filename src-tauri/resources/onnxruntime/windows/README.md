# Windows native runtime

Currently staged from official packages:

- `onnxruntime/windows/onnxruntime.dll`
- `onnxruntime/windows/onnxruntime_providers_shared.dll`
- `onnxruntime/windows/DirectML.dll`

Source packages:

- `Microsoft.ML.OnnxRuntime.DirectML 1.24.4`
- `Microsoft.AI.DirectML 1.15.4`

These files are used by the Rust desktop scanner runtime on Windows with the
`DirectML -> CPU` execution provider order.
