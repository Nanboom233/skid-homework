# 2026-04-03 Scanner YOLO Live Preview IPC Structural Execution Plan

## Internal Grade

`L`

原因：

- 这是一次单条热路径的结构修复，不需要并行 agent wave
- 但涉及 TS ↔ Tauri ↔ Rust stream session 边界，需要显式治理和验证

## Root-Cause Hypothesis

当前 live preview 的 native YOLO 检测虽然已经只有一个 in-flight request，但每个 detect tick 仍会：

1. 从 preview stream 解码得到 `ImageData`
2. 再把整块 `RGBA` 像素通过 `invoke("tauri_scanner_detect_document")` 发回 Rust
3. Rust 再重建图像并继续推理

这条“预览帧先到 JS，再把大块像素回传 Rust”的往返边界，会在 detect 激活时额外占用 Tauri IPC / JS 主线程预算，从而重新放大 `pollWaitMs / ipc wait`。

## Execution Steps

1. 在 `src-tauri/src/stream_decoder.rs` 增加 latest preview frame cache，并让其与 stream start/stop/session 失效联动。
2. 在 `src-tauri/src/scanner_detect.rs` 为 `ScannerDetectDocumentRequest` 增加 live preview cache 输入模式，并实现 preview packet -> image 的恢复逻辑。
3. 在 `src/lib/tauri/scanner-detect.ts` 增加一个显式的 “use latest preview frame” detect API。
4. 在 `src/components/scanner/ScannerView.tsx` 只对 live preview native detect 切到 cached preview frame 输入；仍保留 captured redetect 的 RGBA/source 路径。
5. 补 Rust 单测，验证 preview packet 恢复和 latest preview cache 取用逻辑。
6. 跑 verification：
   - `pnpm exec tsc --noEmit --pretty false`
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   - `cargo check --manifest-path src-tauri/Cargo.toml`
   - `git diff --check`

## Ownership Boundaries

- `src-tauri/src/stream_decoder.rs`: preview latest-frame cache 生命周期
- `src-tauri/src/scanner_detect.rs`: cached preview packet 取用与解码
- `src/lib/tauri/scanner-detect.ts`: JS bridge API
- `src/components/scanner/ScannerView.tsx`: live preview detect 调用切换
- `docs/*` / `outputs/runtime/*`: requirement/plan/proof artifacts

## Rollback Rules

若 live preview detect 出现空帧、错帧或 stop/restart 后命中陈旧 packet：

1. 回退 `stream_decoder.rs` 的 latest preview cache
2. 回退 `ScannerView.tsx` 到原 RGBA invoke 路径
3. 保留本轮文档与测试作为后续继续分析依据

## Phase Cleanup Expectations

1. 不保留临时 benchmark 进程。
2. 输出本轮 `phase-execution.json` 与 `cleanup-receipt.json`。
3. 文档中明确说明这次修复针对的是 live preview detect IPC 边界，而不是 preview renderer backlog 或参数调优。
