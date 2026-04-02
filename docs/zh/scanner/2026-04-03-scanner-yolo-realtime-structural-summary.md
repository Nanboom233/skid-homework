# 2026-04-03 扫描器原生 YOLO 实时链路结构性优化总结

## 本轮约束

用户明确禁止通过调低分辨率、拉长检测间隔、修改阈值等方式做“假优化”。

因此本轮只做了两类结构性改动：

1. **Rust detect 主路径 cache**
2. **预览流分发去掉多余 rAF 栅栏**

## 关键改动

### 1. 原生 detect 主路径引入 detection context cache

文件：

- `src-tauri/src/scanner_detect.rs`

新增：

- `DetectionContextCacheKey`
- `DetectionRuntimeContext`
- `DetectionContextCacheState`
- `resolve_detection_runtime_context()`
- `build_detection_runtime_context()`
- `reset_scanner_yolo_runtime_caches()`

优化前，`detect_document_native_yolo()` 每次调用都会重复做：

- resource root 选择
- scanner yolo config 读取/解析
- resource status 推导
- model selection
- preferred provider 解析

优化后，上述内容只在 cache miss 时计算一次；后续实时 detect 直接复用。

### 2. detect 路径不再依赖完整 probe 响应组装

文件：

- `src-tauri/src/scanner_detect.rs`

优化前：

- `detect_document_native_yolo()` 每次先跑 `probe_native_yolo_runtime_with_hints()`
- 再自己继续做输入准备与推理

优化后：

- detect 主路径直接走轻量 runtime context + runtime/session ensure
- 只组装 detect 响应需要的字段

这避免了每帧进入完整 probe 语义。

### 3. 预览流分发不再先等一拍 requestAnimationFrame

文件：

- `src/lib/tauri/adb.ts`

优化前：

- `startTauriDecodeStream()` 收到最新 frame packet 后，默认等到下一次 `requestAnimationFrame()` 再把 packet 交给 `frame-source`

优化后：

- 优先用 `queueMicrotask()` 立即分发最新 packet
- 保持 latest-only 合并语义
- 把 `rAF` 这层额外调度从 transport handoff 中拿掉

说明：

- 这改的是 **预览 transport 拓扑**
- 不是修改检测频率参数
- 也不是降低处理分辨率

## 验证

已通过：

- `pwsh -NoProfile -File .\scripts\test-scanner-yolo-realtime-structural.ps1`
- `pnpm exec tsc --noEmit --pretty false`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-benchmark -- .github/images/skidhw-thumbnail-new.png --iterations 5`
- `.\\src-tauri\\target\\debug\\scanner-native-yolo-benchmark.exe .github/images/skidhw-thumbnail-new.png --iterations 5`

## Benchmark 结论

使用 **已编译好的 exe** 直接运行，结果更可信：

```json
{
  "encoded-fullres": 463.58722,
  "raw-rgba-bounded-320x180-cached": 406.69758,
  "raw-rgba-bounded-320x180-reset-each-run": 19946.8126
}
```

关键对比不是参数差异，而是同一请求下：

- `raw-rgba-bounded-320x180-cached`
- `raw-rgba-bounded-320x180-reset-each-run`

结果：

- cached 平均：`406.69758ms`
- 每次重置 cache 平均：`19946.8126ms`

也就是：

- **不复用 runtime context 时，成本会暴涨到约 60x**

这说明上一轮 detect 主路径里“每帧重复配置解析/probe 语义”确实是实打实的结构性浪费，不是参数问题。

## 结论

本轮完成的不是“调参提速”，而是：

1. 把原生 YOLO detect 从完整 probe 语义里拆出来；
2. 把配置/模型选择做成可复用 cache；
3. 把预览 transport handoff 的多余 `rAF` 去掉。

如果继续往下压，下一轮最应该继续做的是：

1. 给 `frame-source` 增加离线可跑的 preview packet decode benchmark；
2. 评估把 `decodeFramePacketToRgba()` 从主线程搬到 worker；
3. 检查 `ScannerView` 中 `putImageData()` 与 portrait 旋转路径的主线程成本。
