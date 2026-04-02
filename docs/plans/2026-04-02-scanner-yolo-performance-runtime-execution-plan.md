# Scanner YOLO Runtime Performance Execution Plan

## Goal

把桌面端原生 YOLO 预览检测从“高开销编码/解码路径”切到“低延迟原始像素路径”，并同步削减 Rust 侧无谓的后处理成本。

## Internal Grade

`L`

原因：

- 任务是单条主链路性能优化；
- 变更集中在前端桥接与 Rust runtime；
- 当前运行受 `cunzhi` 约束，不使用 sub-agent 做最终交付。

## Repo-grounded Facts

- 预览检测调用点：`src/components/scanner/ScannerView.tsx`
- 原生桥接：`src/lib/tauri/scanner-detect.ts`
- Rust runtime：`src-tauri/src/scanner_detect.rs`
- 现有原生 PNG 编码桥：`src-tauri/src/png_bridge.rs`
- 预览处理尺寸常量：
  - `CV_MAX_WIDTH = 320`
  - `CV_MAX_HEIGHT = 180`

## Execution Waves

### Wave 1: 去掉预览原生检测的 PNG 往返

目标：

- 预览检测改为直接发送 RGBA + 宽高；
- 保留旧的字节流检测路径给文件/兼容场景。

变更点：

- `src/lib/tauri/scanner-detect.ts`
- `src/components/scanner/ScannerView.tsx`
- `src-tauri/src/scanner_detect.rs`

### Wave 2: 让 maxWidth / maxHeight 真正生效

目标：

- Rust 对输入图像做按比例缩放到限制尺寸内；
- 推理后把点位映射回原始输入尺寸。

变更点：

- `src-tauri/src/scanner_detect.rs`

### Wave 3: 压缩 heatmap 后处理成本

目标：

- 不再把每个 heatmap plane 放大回原图再跑连通域；
- 在 heatmap 空间直接算连通域/重心，再映射回目标尺寸。

变更点：

- `src-tauri/src/scanner_detect.rs`

### Wave 4: 验证与收据

目标：

- 运行 TS/Rust 校验；
- 提供原生 YOLO 冒烟/性能证据；
- 写入 phase / cleanup receipt。

## Ownership Boundaries

- 前端负责：
  - 选择低延迟原生检测入口；
  - 维持现有 UI debug 语义；
  - 不改变 capture / post-process 业务契约。

- Rust 负责：
  - 原生检测请求兼容两种输入形态；
  - 真正执行低成本预处理和后处理；
  - 不破坏 probe / session / config 绑定。

## Verification Commands

```powershell
pnpm exec tsc --noEmit --pretty false
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke -- .github/images/skidhw-thumbnail-new.png
git diff --check
```

## Rollback Rules

- 若新原始像素路径在 Tauri 环境不可用，保留旧字节流路径作为兼容回退；
- 若 heatmap 空间解码造成明显回归，优先保留正确性，再局部回退该部分实现；
- 不回滚用户已有 `.gitignore` 变更。

## Cleanup Expectations

- 写入：
  - `phase-requirement-doc.json`
  - `phase-plan.json`
  - `phase-execution.json`
  - `cleanup-receipt.json`
- 不遗留临时 benchmark 文件；
- 不新增未引用资源。
