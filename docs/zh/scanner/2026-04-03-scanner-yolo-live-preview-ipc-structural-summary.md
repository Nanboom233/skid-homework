# 2026-04-03 Scanner YOLO Live Preview IPC Structural Summary

## 结论

这轮剩余瓶颈的结构性根因，不再是 preview renderer backlog，而是 live preview 的 native detect 仍在走：

`JS ImageData RGBA -> tauri_scanner_detect_document -> Rust 重建图像`

也就是每个 detect tick 都会把当前预览帧的大块 `RGBA` 像素重新穿过一次 Tauri IPC。

这条边界会在 native detect 激活时额外占用 JS 主线程 / IPC 预算，重新把 `pollWaitMs / ipc wait` 顶高。

## 本轮修复

### 1. Rust stream decoder 持有 latest preview frame packet

文件：

- `src-tauri/src/stream_decoder.rs`

改动：

- 增加 Rust 侧 latest preview frame packet cache；
- stream start / stop / session 结束时显式清空；
- 每次成功生成 preview packet 后，更新 cache。

结果：

- live preview detect 不再必须依赖前端把 `ImageData` 再传回 Rust。

### 2. Native detect 支持直接消费 cached preview frame

文件：

- `src-tauri/src/scanner_detect.rs`

改动：

- `ScannerDetectDocumentRequest` 新增 `useLatestPreviewFrame`；
- `resolve_detect_input_image()` 支持直接从 Rust 侧 cached preview packet 取输入；
- 新增 preview packet I420 解析与 `I420 -> RgbImage` 恢复逻辑；
- detect response 新增 `inputTransport`，便于在前端 debug 面板确认命中哪条输入路径。

结果：

- live preview 路径可以直接吃 Rust 侧最近预览帧；
- captured redetect / source-bytes / RGBA 路径继续保留。

### 3. ScannerView 只把 live preview detect 切到 cached preview path

文件：

- `src/components/scanner/ScannerView.tsx`
- `src/lib/tauri/scanner-detect.ts`

改动：

- 新增 `detectDocumentWithTauriNativeYoloLatestPreview()` bridge；
- preview CV loop 调用 native-yolo 时改为优先走 latest preview cache；
- captured / post-process redetect 仍走原有图像输入路径；
- 若 cached preview path 单次异常，本 tick 退回一次 RGBA invoke，避免首帧/偶发 cache miss 直接误伤功能。

## 为什么这比继续调参更对

用户已经明确否决：

- 降检测分辨率
- 拉长检测间隔
- 调阈值
- 调稳定参数

本轮做的是把热路径里“重复跨 IPC 回传 RGBA”这条结构性成本切掉，而不是把体验退化成“少算一点”。

## 验证

执行：

- `pnpm build`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo build --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke`
- `.\src-tauri\target\debug\scanner-native-yolo-smoke.exe .github/images/skidhw-thumbnail-new.png --raw-rgba --max-width 320 --max-height 180`
- `git diff --check`

补充：

- `scripts/test-scanner-yolo-live-preview-ipc-structural.ps1` 已更新为包含 typecheck / frontend build / Rust check / Rust test / native smoke build / smoke run / diff check 的闭环脚本；
- 在当前沙箱里 `pnpm build` 会命中 `spawn EPERM`，因此实际 build 是在沙箱外重新执行完成的；
- Rust 单测里新增了 latest preview cache 输入路径覆盖，确认 cached preview packet 能被 native detect 正常取用。

结果：

- `pnpm build` 通过
- Rust tests 通过：`14 passed`
- `cargo build --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-smoke` 通过
- smoke run 通过，输出中已包含 `inputTransport: "rgba-ipc"`，说明最终二进制已带上本轮新增字段
- `git diff --check` 通过

## 仍需真实设备复测

这轮已经把 live preview native detect 的额外 RGBA IPC 往返切掉，但还没有在真机上重新采样：

- `pollWaitMs`
- `jsDecodeMs`
- `canvasDrawMs`

下一步应在真实设备上确认：

1. cached preview detect 命中后，`pollWaitMs` 是否明显收敛；
2. 如果仍有明显峰值，再继续看 preview render upload；
3. 若还剩 native detect 成本，再考虑更深的 preview packet / tensor 直通，而不是回到参数调优。
