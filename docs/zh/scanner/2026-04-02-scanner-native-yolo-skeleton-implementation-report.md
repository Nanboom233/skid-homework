# 2026-04-02 Native YOLO Rust / ORT 实施报告

## 本轮目标

先收尾已中断的 Rust 接线工作，恢复工程可编译性，并把 Native YOLO 的桌面骨架落实为：

- Tauri 可调用的 `probe` / `detect` 命令
- 可执行的 CLI probe
- 前端 TS bridge
- 资源路径与打包位点
- 最小测试脚本与编译/运行验证

本轮**不伪造真实推理能力**，因为当前仍没有可直接交付的模型与 ORT runtime 资源。

但和上一版不同的是：

- ORT crate 已真实接入
- 已具备动态库加载与 session 初始化代码路径
- probe 不再只是“看文件”，而是能真实报告 ORT runtime / session 状态

## 已完成

### 1. Rust / Tauri 原生命令与 ORT 运行时

新增：

- `src-tauri/src/scanner_detect.rs`

已提供命令：

- `tauri_scanner_probe_yolo`
- `tauri_scanner_detect_document`

当前语义：

- `probe`：真实探测资源路径、provider 偏好、ORT runtime 是否加载成功、session 是否可建立
- `detect`：可解码输入图像并返回诚实的 runtime/session 状态，但**不做实际推理**

本轮新增的核心能力：

- 接入 `ort = 2.0.0-rc.12`
- 使用 `load-dynamic + api-24`
- 已实现进程级 runtime state
- 已实现 model session 的缓存与重试路径
- 已实现 Windows `DirectML -> CPU` / Linux `TensorRT -> CUDA -> CPU` 的 provider 顺序

### 2. CLI probe

新增：

- `src-tauri/src/bin/scanner-native-yolo-probe.rs`

用途：

- 独立于前端验证资源解析与 probe 输出
- 便于后续 CI / 本地排查 runtime 打包问题

### 3. 前端桥接

新增：

- `src/lib/tauri/scanner-detect.ts`

已提供：

- `probeTauriScannerYolo()`
- `detectDocumentWithTauriNativeYolo()`

### 4. 设置骨架

修改：

- `src/store/settings-store.ts`

新增持久化字段：

- `scannerDetectionBackend: "opencv" | "native-yolo"`
- `scannerNativeYoloStrictMode: boolean`

这一步先把“可选后端”的状态位补齐，但**尚未接入 Scanner UI 和实时检测分发**。

### 5. 资源位与打包位

新增：

- `src-tauri/resources/scanner-yolo-config.json`
- `src-tauri/resources/models/README.md`
- `src-tauri/resources/onnxruntime/windows/README.md`
- `src-tauri/resources/onnxruntime/linux/README.md`

修改：

- `src-tauri/tauri.conf.json`

修正点：

- 原先只有 `resources/*`
- 现在显式包含嵌套目录，避免后续 `models/**`、`onnxruntime/**` 不被打包

### 6. 测试脚本

新增：

- `scripts/test-scanner-native-yolo-skeleton.ps1`

## 本轮验证

### TypeScript

已执行：

```powershell
pnpm exec tsc --noEmit --pretty false
```

结果：通过。

### Rust 编译

已执行：

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

结果：通过。

### Rust 测试

已执行：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

结果：

- 7 passed
- 0 failed

说明：

- 通过了既有 `scanner_postprocess` 测试
- 新增 `scanner_detect` 基础测试通过

### 运行 probe

已执行：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe
```

关键输出结论：

- 平台：`windows`
- 目标平台策略：`windows-directml`
- 当前实际 provider：`CPU`
- `runtimeReady = false`
- `modelReady = false`
- `sessionReady = false`
- `detectionImplemented = false`
- `runtimeError = Missing ONNX Runtime library`
- `sessionError = Model file is missing`

这说明：

- 路径探测逻辑已工作
- ORT 真接入代码路径已工作
- probe 已从“文件存在性检查”升级成“真实 runtime/session 状态检查”
- 没有把缺模型/缺 runtime 伪装成“功能已完成”

## ORT 接入细节

### 1. 依赖与配置

已在 `src-tauri/Cargo.toml` 中接入：

```toml
ort = { version = "2.0.0-rc.12", default-features = false, features = ["std", "load-dynamic", "api-24", "directml", "cuda", "tensorrt"] }
```

补充说明：

- `api-24` 是必要项
- 如果只开 `load-dynamic` 而漏掉 API 特性，会在 `ort` crate 内部触发 EP API 编译错误

### 2. 已实现的真实运行时闭环

当前 Rust 侧已具备：

1. 从资源目录解析 ORT 动态库路径
2. 调用 `ort::init_from(...)` 做动态加载
3. 读取 ORT build info
4. 用当前平台 provider 顺序构建 `SessionBuilder`
5. 在模型存在时尝试 `commit_from_file(...)`
6. 将 runtime / session 错误回传给 probe 与 detect

### 3. 当前为什么仍然是 false

当前不是代码没接上，而是资源确实没到位：

- Windows 缺 `onnxruntime/windows/onnxruntime.dll`
- 模型缺 `models/document-boundary-yolo-pose.onnx`

所以现在的 `runtimeReady = false` 和 `sessionReady = false` 是**真实结果**。

## 研究修正结论

### 1. 公开模型不是完全没有，但大多不满足最终契约

通过本轮 `grok-search` 重新核实后，结论应修正为：

- **公开的 YOLO document detector / scanner 示例是存在的**
- 但它们大多输出的是：
  - 单类 document bbox
  - 或“粗定位 + OpenCV 后处理”
- 它们通常**不是直接输出 4 个有序角点**

因此：

- 可以把公开 bbox 模型当作**低门槛 baseline**
- 但不能把它当作“可直接替代四角点模型”的主计划

### 2. 主路线仍应是低成本自训练

当前最稳妥主线仍然是：

```text
YOLO Pose（4 keypoints）
+ synthetic compositing
+ MIDV-500 / MIDV-2019 转换与补强
+ ONNX export
```

原因：

- 与扫描器下游需要的四角点契约直接一致
- MIDV-500 / MIDV-2019 已有四边形标注，可直接转换
- 比 segmentation 更轻
- 比 OBB 更符合透视下的一般四边形

### 3. 运行时部署约束

基于本轮重新核对，部署策略应更明确：

- Windows：
  - 优先 `DirectML`
  - 需要 `onnxruntime.dll`
  - 还要准备 `DirectML.dll`
  - 可做成相对自包含的桌面分发
- Linux：
  - 优先 `TensorRT -> CUDA`
  - 不应把完整 NVIDIA 运行时当成随包资源硬塞进仓库
  - 更现实的是：
    - 仅打包我们自己的配置与模型
    - 把 CUDA / TensorRT 作为系统前置条件检查项

## 当前边界

本轮**尚未完成**：

- Native YOLO 真推理
- ScannerView 实时切换接入
- UI availability 展示
- Windows / Linux 实际 runtime 资源落盘与安装策略收尾

也就是说，当前阶段已经不是“骨架未接上”，而是“运行时主线已接上，资源与模型仍缺”。

## 下一步建议

### P0

把 Windows ORT 资源真正放入：

- `onnxruntime/windows/onnxruntime.dll`
- `onnxruntime/windows/DirectML.dll`

然后验证：

- `runtimeReady = true`
- `preferredProvider = DirectML` 或明确回落到 CPU

### P1

把 `scannerDetectionBackend` 真正接入：

- `ScannerControls.tsx`
- `ScannerView.tsx`
- `ScannerDebugPanel.tsx`

### P2

模型路线并行推进：

1. 先验证一个公开 bbox baseline
2. 同步启动 `YOLO Pose 4-corner` 低成本训练
3. 用 public baseline 作为对照，而不是最终目标

## 关联文件

- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/bin/scanner-native-yolo-probe.rs`
- `src/lib/tauri/scanner-detect.ts`
- `src/store/settings-store.ts`
- `src-tauri/tauri.conf.json`
- `scripts/test-scanner-native-yolo-skeleton.ps1`
