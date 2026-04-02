# 2026-04-02 扫描器 YOLO 方案修订总结

## 本轮修订原因

用户补充了两个关键前提：

1. **当前没有可直接使用的 YOLO 模型**
2. **YOLO 不应继续按浏览器方案设计，而应改为桌面版 Rust 原生能力**

因此，旧版“前端 worker + ORT Web”设计已失效，本轮已回到 plan 阶段重做方案。

## 修订后的核心结论

### 1. 架构方向

- Web：继续使用现有 OpenCV
- Desktop / Tauri：新增 Native YOLO
- Native YOLO 位于 Rust 原生侧，而非浏览器

### 2. 平台策略

- **Windows：DirectML -> CPU**
- **Linux：TensorRT -> CUDA -> CPU**

### 3. 推理引擎

推荐：

- **ONNX Runtime**
- **Rust `ort` crate**

不推荐：

- 浏览器侧 ORT Web
- `tract` 作为主方案

### 4. 模型现实

需要修正成更细的版本：

- 存在公开的 YOLO document detector / document scanner 示例；
- 但它们大多是**单类 bbox 粗定位**；
- 不是直接输出四角点的最终扫描器模型。

因此：

- **公开模型可以作为 baseline**
- **不能把“下载公共模型”当主计划**
- 主计划仍应是低成本自训练的四角点方案

### 5. 推荐模型路线

主路线建议：

```text
YOLO Pose（4 keypoints）
+ synthetic page compositing
+ MIDV-500 / MIDV-2019 补强
+ SmartDoc 验证
```

补充修正：

- **不再推荐 OBB 作为主模型路线**
- 原因不是 OBB 没有 4 点，而是它描述的是**旋转矩形**
- 扫描器页面在透视下通常是**一般四边形**
- 因此更合理的模型主线是：
  - **YOLO Pose（4 corners）**：工程成本最低、与 MIDV 标签最匹配
  - **Segmentation + quad fitting**：几何更强、但工程更重

### 6. 数据集判断

适合：

- MIDV-500
- MIDV-2019
- SmartDoc

不适合直接做扫描器边界主训练集：

- DocLayNet
- PubLayNet

### 7. 两阶段链路结论

最新补充结论是：

- **阶段一：YOLO 做定位与粗裁切**
- **阶段二：CV / geometry 做透视精修、轻度展平、OCR 复用**

这意味着 YOLO 不应被设计成“一步直接输出最终 OCR-ready 页面”的万能模块。

## 仓库中的实际接入点

- 前端检测入口：`src/components/scanner/ScannerView.tsx`
- 已有 Tauri 原生桥：`src/lib/tauri/scanner.ts`
- 已有 Rust 后处理命令：`src-tauri/src/scanner_postprocess.rs`
- 已有 Tauri 资源打包：`src-tauri/tauri.conf.json`

结论：应沿用当前 Tauri 原生命令风格，新增 `scanner_detect.rs`，而不是再造一个前端 YOLO worker。

## 本轮已完成

- 修订需求文档：
  - `docs/requirements/2026-04-02-scanner-yolo-optional-backend.md`
- 修订设计文档：
  - `docs/plans/2026-04-02-scanner-yolo-optional-backend-design.md`
- 修订执行计划：
  - `docs/plans/2026-04-02-scanner-yolo-optional-backend-execution-plan.md`
- 新增中文总结：
  - `docs/zh/scanner/2026-04-02-scanner-yolo-optional-backend-summary.md`
- 新增模型方案深挖：
  - `docs/zh/scanner/2026-04-02-scanner-yolo-model-scheme-deep-dive.md`
- 新增两阶段链路分析：
  - `docs/zh/scanner/2026-04-02-scanner-two-stage-pipeline-analysis.md`

## 补充更新：骨架实现已开始

在本总结写完后，已继续完成第一轮 Rust 骨架收尾，新增：

- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/bin/scanner-native-yolo-probe.rs`
- `src/lib/tauri/scanner-detect.ts`
- `scripts/test-scanner-native-yolo-skeleton.ps1`

并已完成：

- `pnpm exec tsc --noEmit --pretty false`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe`

当前 probe 结果表明：

- 配置文件已能被发现
- 模型文件仍缺失
- Windows ORT runtime 仍缺失
- skeleton 已可诚实报告 `runtimeReady = false`

详细实施记录见：

- `docs/zh/scanner/2026-04-02-scanner-native-yolo-skeleton-implementation-report.md`

## 下一步建议

建议下一轮从 **Rust 运行时骨架** 开始，而不是先写模型训练代码：

1. 新增 `src-tauri/src/scanner_detect.rs`
2. 建立 provider probe
3. 固定资源路径与模型配置
4. 完成前端 desktop-only 开关
5. 再并行推进模型路线

## 参考链接

- ORT Rust bindings: https://github.com/open-runtime/ort-rust
- DirectML EP: https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- TensorRT EP: https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- CUDA EP: https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
- MIDV-500: https://arxiv.org/abs/1507.00390
- MIDV-2019: https://arxiv.org/pdf/1910.03184
