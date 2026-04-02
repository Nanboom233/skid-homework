# 2026-04-02 Native Scanner Runtime 真接入报告

## 结论

本轮已经从“YOLO 骨架”推进到“桌面原生 runtime + 公共 baseline 真推理”：

- Windows 官方 ORT + DirectML 运行时已落盘
- 公共四角点模型 `DocAligner fastvit_sa24` 已落盘
- Rust / Tauri 已可真实跑 ONNX 推理并返回 4 个角点
- probe 会明确报告当前实际加载的是 `public-baseline` 还是未来的 `planned-primary YOLO`

这意味着：

- `native-yolo` 这个可选后端已经不再只是 session probe
- 当前可作为桌面版 CV 替代 baseline 使用
- 但最终主路线仍然是用户要求的 Rust + YOLO + Windows DirectML / Linux TensorRT|CUDA

## 2026-04-02 续作更新

本次 `resume` 后，又补齐了三件关键工作：

### 1. Scanner UI 已真正接上 native backend

当前前端已完成以下接入：

- `ScannerControls` 支持选择：
  - `opencv`
  - `native-yolo`
- `SettingsPage` 支持持久化：
  - `scannerDetectionBackend`
  - `scannerNativeYoloStrictMode`
- `ScannerView` 已具备：
  - native probe 刷新
  - backend requested / active 区分
  - strict mode 下阻止 OpenCV fallback
  - debug state 回写当前 provider / model / backend message

因此现在的 `native-yolo` 已不只是“探测按钮”，而是 Scanner 运行链路中的可选 stage-1 backend。

### 2. Windows DirectML session 约束已显式写入 Rust runtime

结合本轮通过 `grok-search` 复核的 ONNX Runtime 官方文档：

- DirectML 文档：
  - https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html

Rust 侧已显式补上 DirectML 安全约束：

- `with_parallel_execution(false)`
- `with_memory_pattern(false)`

对应修改位置：

- `src-tauri/src/scanner_detect.rs`

同时，当前实现本来就通过进程级 `Mutex<OrtRuntimeState>` 把同一 session 的 `Run()` 串行化，因此也满足 DirectML “同一 inference session 不支持并发 Run” 的官方限制。

### 3. Tauri 壳已真实拉起

本轮实际执行了：

```powershell
pnpm tauri:dev
```

结果不是只停留在编译通过，而是已经观察到桌面进程窗口：

- process name: `skid-homework`
- window title: `Skid Homework`

这说明：

- 当前改动没有把桌面壳启动链路打坏
- 但由于本轮没有桌面自动化去操作真实扫码流，**“手工点击切换 backend + 实机扫码” 仍未完成最终交互验收**

## 官方资料复核后的设计收束

以下结论是本轮基于 `grok-search` 与官方源复核后的工程判断。

### A. Windows 路线仍然应是 DirectML，而不是回退到浏览器推理

官方 DirectML 文档确认：

- DirectML 仍受支持，但处于 sustained engineering
- 要求 Windows 10 1903+ / Windows 11
- 要求 DX12-capable hardware
- 不支持 mem pattern optimization
- 不支持同一 session 的并发 `Run`

这并不推翻当前方案，反而说明：

> 对桌面版扫描器来说，Windows 继续走 Rust + ORT + DirectML 仍是对的，但 runtime builder 必须按官方约束配置。

### B. Linux 路线应明确写成 TensorRT -> CUDA -> CPU

复核来源：

- TensorRT EP：
  - https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- CUDA EP：
  - https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html

当前可确认的部署判断：

- Linux 应优先尝试 `TensorRT`
- unsupported nodes / subgraphs 应允许回退到 `CUDA`
- 最终再回到 `CPU`
- TensorRT engine / timing cache 有价值，但**不可假设可跨机器、跨驱动、跨 TRT / CUDA 版本复用**

因此：

> Linux 打包策略不应承诺“把完整 NVIDIA 运行时完全随仓库静态兜住”，而应把 CUDA / TensorRT 视为目标机前置依赖。

### C. 真正的 YOLO 主路线应是 Pose 4-corner，而不是 OBB

复核来源：

- Pose task:
  - https://docs.ultralytics.com/tasks/pose/
- Export:
  - https://docs.ultralytics.com/modes/export/

当前确认的事实：

- Ultralytics pose 支持自定义 keypoints
- 可训练 custom pose dataset
- 可导出 ONNX / TensorRT
- pose 输出天然更接近当前扫描器要的四角点契约

因此最终主路线仍应是：

```text
YOLO Pose（4 keypoints）
-> ONNX export
-> Rust decoder / postprocess
-> Point[] | null
```

### D. 用户目前没有可用 YOLO model 时，最诚实的交付策略不是硬装假 YOLO

本轮对公开模型 / 数据路线的复核结论是：

- 当前仓库已具备一个可公开落盘、可真实推理的 baseline：
  - `docaligner-fastvit-sa24`
- 公开数据方面，MIDV-500 是强主源：
  - Smart Engines whitepaper:
    - https://smartengines.com/wp-content/uploads/2020/04/datasets-of-id-documents-midv-500.pdf
  - arXiv:
    - https://arxiv.org/abs/1807.05786
- MIDV-500 明确提供：
  - 500 videos
  - 15,000 frames
  - 每帧 `quad` 四角点 JSON
- 基于本轮检索结果的工程判断：
  - **没有找到一个权威、可直接下载、且输出“4 个有序文档角点”的公共 YOLO pose 成品模型**

所以当前正确设计是：

1. **现在交付**
   - ORT runtime
   - public baseline
   - native backend UI / debug / strict mode

2. **下一阶段主线**
   - synthetic compositing
   - MIDV-500 / MIDV-2019 转 pose 标签
   - 训练 `kpt_shape: [4, 3]` 的 YOLO pose 小模型
   - export ONNX / TensorRT
   - 接回当前 Rust runtime

## 为什么这次不是“假接入”

通过 `grok-search` 和官方源核实后，当前接入的是两类资源：

### 1. Windows 官方 runtime

来源：

- `Microsoft.ML.OnnxRuntime.DirectML 1.24.4`
- `Microsoft.AI.DirectML 1.15.4`

当前已落入：

- `src-tauri/resources/onnxruntime/windows/onnxruntime.dll`
- `src-tauri/resources/onnxruntime/windows/onnxruntime_providers_shared.dll`
- `src-tauri/resources/onnxruntime/windows/DirectML.dll`

### 2. 公共四角点 baseline 模型

来源：

- `DocsaidLab/DocAligner`
- 公共模型：`fastvit_sa24_h_e_bifpn_256_fp32.onnx`

当前已落入：

- `src-tauri/resources/models/docaligner-fastvit_sa24.onnx`

备注：

- 这不是 YOLO
- 但它是公开可下载、可商用（Apache-2.0）、输出四角点的真实公共模型
- 适合作为当前桌面版 stage-1 baseline

## 代码改动

### Rust 侧

核心文件：

- `src-tauri/src/scanner_detect.rs`

本轮新增能力：

- 资源解析支持“双模型位”：
  - `docaligner-fastvit-sa24`
  - `document-boundary-yolo-pose-4pt`（规划位）
- probe 响应新增：
  - `selectedModelId`
  - `selectedModelKind`
  - `selectedModelTask`
  - `selectedModelPath`
- Windows 资源检查新增：
  - `onnxruntime_providers_shared.dll`
- `detect_document_native_yolo` 已实现真实推理路径：
  - 输入图像解码
  - `256x256` resize
  - RGB -> BGR planar
  - ORT session.run(...)
  - `heatmap` 输出解码
  - 最大连通域质心 -> 4 角点

### CLI / 调试

新增：

- `src-tauri/src/bin/scanner-native-yolo-smoke.rs`

用途：

- 给任意图片跑一次本地 smoke inference
- 验证当前资源 + 模型 + 解码链路是否全通

### 前端类型桥接

修改：

- `src/lib/tauri/scanner-detect.ts`

补充了当前实际加载模型的元信息字段，方便 Scanner UI 后续显式展示：

- 当前是 public baseline
- 还是未来 YOLO 主模型

### 资源配置说明

修改：

- `src-tauri/resources/scanner-yolo-config.json`

现在区分：

- `intendedPrimaryModel`
- `activePublicBaseline`

避免再把公共 baseline 伪装成“已经有 YOLO 模型”。

## 验证结果

### 1. TypeScript

执行：

```powershell
pnpm exec tsc --noEmit --pretty false
```

结果：通过。

### 2. Rust 编译

执行：

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

结果：通过。

### 3. Rust 测试

执行：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

结果：8 passed, 0 failed。

### 4. Native probe

执行：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe
```

关键结论：

- `preferredProvider = DirectML`
- `runtimeReady = true`
- `preferredProviderReady = true`
- `selectedModelId = docaligner-fastvit-sa24`
- `modelReady = true`
- `sessionReady = true`
- `detectionImplemented = true`

这说明：

- Windows DirectML runtime 已真实可用
- 公共 baseline 模型 session 已真实建立
- 当前并不是 CPU fallback 假成功

### 4.1 Native probe（二次复验）

在 `resume` 后再次执行：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe
```

本次复验结果仍然是：

- `preferredProvider = DirectML`
- `runtimeReady = true`
- `preferredProviderReady = true`
- `selectedModelId = docaligner-fastvit-sa24`
- `sessionReady = true`
- `detectionImplemented = true`

说明本轮补上的 DirectML session builder 约束没有破坏现有 runtime。

### 5. End-to-end smoke inference

执行：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke -- <DocAligner sample image>
```

实际输出：

```json
{
  "selectedModelId": "docaligner-fastvit-sa24",
  "preferredProvider": "DirectML",
  "points": [
    {"x": 48.02727, "y": 223.61636},
    {"x": 387.05945, "y": 198.10631},
    {"x": 422.9337, "y": 345.48566},
    {"x": 40.19384, "y": 361.41306}
  ]
}
```

这些数值与 DocAligner 文档中的公开示例基本一致，说明 Rust 侧预处理和后处理方向正确。

## 设计判断更新

### 当前最优实现判断

用户目前没有可用 YOLO 模型，因此最合理的两阶段设计是：

1. 当前交付：
   - Rust + ORT + DirectML
   - DocAligner public baseline
   - 让桌面版先具备真实 stage-1 能力

2. 后续主线：
   - Rust + ORT
   - YOLO Pose 4-corner
   - Windows DirectML
   - Linux TensorRT -> CUDA

### 为什么没有直接“网上找个 YOLO 模型”

`grok-search` 复核后的结论是：

- 公开的 document YOLO 示例很多只给 bbox
- 截至 2026-04-02，没有一个成熟、公开、现成可下载的“四角点 YOLO 模型”可直接替代我们的契约

所以当前正确做法不是硬装一个假 YOLO，而是：

- 诚实使用公共四角点 baseline
- 同时保留 YOLO 为最终主路线

## 后续建议

### P0

补完真实交互验收：

- 打开 Scanner
- 切换 backend selector
- 验证 strict mode 下确实阻止 OpenCV fallback
- 观察 debug panel 中 requested / active backend、provider、model metadata 是否正确变化

### P1

优化 preview 路径的 native detect IPC 成本：

- 当前 `ScannerView` 会把 preview frame 编码成 PNG 再 invoke 到 Rust
- 这是 correctness-first，但不是最终性能形态
- 下一步优先考虑：
  - raw RGBA / binary bytes 直传
  - 或降低 native detect cadence

### P2

启动最终 YOLO 训练路线：

- synthetic compositing
- MIDV-500 / MIDV-2019 conversion
- YOLO pose 4 keypoints
- ONNX export
- 再接入当前 Rust runtime

### P3

把 Linux 部署合同写成明确文档：

- TensorRT / CUDA 版本前置
- cache 策略
- 不可移植性说明
- CPU fallback 语义

## 关联文件

- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/bin/scanner-native-yolo-probe.rs`
- `src-tauri/src/bin/scanner-native-yolo-smoke.rs`
- `src/lib/tauri/scanner-detect.ts`
- `src-tauri/resources/scanner-yolo-config.json`
- `src-tauri/resources/models/docaligner-fastvit_sa24.onnx`
- `src-tauri/resources/onnxruntime/windows/onnxruntime.dll`
- `src-tauri/resources/onnxruntime/windows/onnxruntime_providers_shared.dll`
- `src-tauri/resources/onnxruntime/windows/DirectML.dll`
