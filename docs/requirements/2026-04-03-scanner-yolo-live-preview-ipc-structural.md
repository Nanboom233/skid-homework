# 2026-04-03 Scanner YOLO Live Preview IPC Structural Requirement

## Background

`2026-04-03` 的 preview backlog 修复已经把 renderer 侧“顺序偿还旧 packet 债务”切掉，但用户在真实实时路径里仍观测到：

- 当桌面端 `native-yolo` 实时检测开始工作时，`pollWaitMs / ipc wait` 仍会飙到 `1000ms+`
- 当前前端预览检测路径仍然会把 `ImageData` 的整块 `RGBA` 像素再次通过 `tauri_scanner_detect_document` 发回 Rust
- 这意味着 live preview 每次 detect 都在做一条额外的 `JS RGBA -> Tauri IPC -> Rust image rebuild` 热路径

本轮目标不是继续调参数，而是继续消除这条结构性跨边界成本。

## Goal

在不靠降分辨率、拉长间隔或调阈值的前提下，消除 live preview 的 native YOLO 检测对 `RGBA` 大 payload IPC 的依赖，收敛实时检测期间的 `ipc wait` 飙升。

## Constraints

1. 不降低检测分辨率来伪装优化。
2. 不增大检测间隔或修改稳定判定参数。
3. 不破坏当前高质量 still capture / post-process redetect 路径。
4. 保留现有 OpenCV fallback、strict mode、probe/debug 行为。

## Non-Goals

1. 不重写 Android camera server。
2. 不重写 H.264 解码器。
3. 不宣称真实设备收益而不给结构性证据。

## Required Structural Change

### 1. Live preview native detect must avoid RGBA re-upload

实时预览检测路径应改为优先复用 Rust 侧最近一次 preview frame packet，而不是每个 detect tick 都把 `ImageData.data` 重新走 `invoke()` 发回 Rust。

### 2. Cached preview frame must stay session-safe

Rust 侧用于 native detect 的 latest preview frame cache 必须随 stream session 生命周期更新，并在 start/stop/失效时清理，避免误用陈旧 packet。

### 3. Non-preview paths must remain available

对 captured still / post-process redetect 等非 live preview 场景，仍保留基于 `sourceBytes` / `rgbaBytes` 的 detect 输入路径。

## Acceptance Criteria

1. `ScannerView` 的 live preview native detect 不再默认走 `detectDocumentWithTauriNativeYoloRgba(frame, ...)`。
2. `tauri_scanner_detect_document` 支持从 Rust 侧 cached preview frame 取输入。
3. cached preview frame 的生命周期与 stream session 对齐，stop/restart 后不会保留脏数据。
4. TypeScript typecheck、Rust test/check 通过。
5. 本轮文档明确记录根因判断：剩余瓶颈来自 live detect 的额外 RGBA IPC 边界，而不是回退到参数调优。
