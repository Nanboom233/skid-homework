# 2026-04-02 扫描器原生 YOLO 运行时性能优化总结

## 本轮目标

压缩桌面端扫描器原生 YOLO 预览检测延迟，重点处理用户反馈的 `ipc wait` 偏高问题。

## 关键改动

### 1. 预览原生检测不再走 PNG 往返

- 前端新增 `detectDocumentWithTauriNativeYoloRgba()`
- `ScannerView` 预览检测改为直接发送 `ImageData` 的 RGBA 字节
- 不再执行：
  - `ImageData -> PNG 编码`
  - `PNG -> invoke`
  - `Rust 再 decode PNG`

旧的字节流检测入口仍保留，供文件/兼容场景使用。

### 2. Rust 原生检测真正使用 maxWidth / maxHeight

- `ScannerDetectDocumentRequest` 新增 RGBA 输入形态：
  - `rgbaBytes`
  - `rgbaWidth`
  - `rgbaHeight`
- Rust 端新增输入预处理：
  - 若请求带尺寸上限，则先按比例缩放到限制尺寸内
  - 推理完成后再把四角点映射回原始输入尺寸

这让前端传入的 `320x180` 预览检测限制终于真实生效。

### 3. heatmap 后处理改为 heatmap 空间解码

旧实现会把每个 heatmap plane 放大回原图，再在原图尺寸上做连通域与质心计算。

本轮改为：

- 直接在 heatmap 空间做连通域
- 直接在 heatmap 空间求质心
- 最后再映射回目标输出尺寸
- 若某通道阈值连通域为空，则回退到 argmax 点，避免直接失败

这部分显著降低了纯 CPU 后处理开销。

### 4. 新增本地 benchmark 入口

新增：

- `src-tauri/src/bin/scanner-native-yolo-benchmark.rs`
- `src-tauri/src/bin/scanner-native-yolo-smoke.rs` 支持：
  - `--raw-rgba`
  - `--max-width`
  - `--max-height`

可直接比较不同输入路径和处理尺寸下的原生 YOLO 运行时成本。

## 验证结果

### 编译/测试

已通过：

- `pnpm exec tsc --noEmit --pretty false`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`

### 原生 YOLO benchmark

命令：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-benchmark -- .github/images/skidhw-thumbnail-new.png --iterations 3
```

结果：

- `encoded-fullres`
  - warmup: `17450.7699ms`
  - average: `649.0459ms`
- `raw-rgba-bounded-320x180`
  - warmup: `365.0768ms`
  - average: `402.0948ms`

按 warm session 平均值对比：

- `649.0459ms -> 402.0948ms`
- 约下降 `38.0%`

### 冒烟

命令：

```powershell
.\src-tauri\target\debug\scanner-native-yolo-smoke.exe .github/images/skidhw-thumbnail-new.png --raw-rgba --max-width 320 --max-height 180
```

结果确认：

- runtime ready = `true`
- session ready = `true`
- preferred provider = `directml`
- working resolution = `320x180`

## 影响范围

实际变更文件：

- `src/lib/tauri/scanner-detect.ts`
- `src/components/scanner/ScannerView.tsx`
- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/bin/scanner-native-yolo-smoke.rs`
- `src-tauri/src/bin/scanner-native-yolo-benchmark.rs`

## 结论

本轮已经把预览原生 YOLO 的最大结构性浪费去掉了两项：

1. PNG 编码/解码往返
2. 原图尺寸 heatmap 后处理

并且把预览处理尺寸真正压到了 `320x180`。

如果后续还要继续压榨，优先级建议是：

1. 在真实扫描预览流里记录 native-yolo 分支的端到端耗时采样；
2. 检查 DirectML provider 下 ORT session 选项是否还有可进一步下探的空间；
3. 若模型允许，评估更小输入尺寸或更小 public baseline。
