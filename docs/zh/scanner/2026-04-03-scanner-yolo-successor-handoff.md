# 2026-04-03 扫描器原生 YOLO 继任者交接说明

## 当前提交状态

- 最新相关提交：`fcc9855`
- 提交信息：
  - `perf(scanner): streamline native yolo runtime (yolo-detection)`
- 这是一次 **GPG signed commit**
- 当前工作区已清理到 `git status --short` 为空

## 本轮已经完成的事情

### 第一轮：去掉明显结构性浪费

已完成：

- 预览原生检测从 `PNG 编码 -> IPC -> Rust 解码` 改成 **直接 RGBA 请求**
- Rust 原生检测真实支持 `maxWidth` / `maxHeight`
- heatmap 后处理改成 **heatmap 空间解码**
- 新增：
  - `src-tauri/src/bin/scanner-native-yolo-smoke.rs` 扩展参数
  - `src-tauri/src/bin/scanner-native-yolo-benchmark.rs`

对应文档：

- `docs/requirements/2026-04-02-scanner-yolo-performance-runtime.md`
- `docs/plans/2026-04-02-scanner-yolo-performance-runtime-execution-plan.md`
- `docs/zh/scanner/2026-04-02-scanner-yolo-performance-runtime-summary.md`

### 第二轮：明确禁止“调参装优化”后的结构性优化

用户明确要求：

> 不允许修改参数的假优化

所以第二轮没有继续动：

- 检测分辨率
- 检测间隔
- 阈值
- provider 顺序

第二轮真正落下的结构性优化只有两项：

1. **`src-tauri/src/scanner_detect.rs`**
   - 给原生 detect 主路径加了 `detection runtime context cache`
   - `detect_document_native_yolo()` 不再每帧重走完整 `probe/config/resource/model selection`

2. **`src/lib/tauri/adb.ts`**
   - 去掉预览帧 handoff 的额外 `requestAnimationFrame` 栅栏
   - 改成 `queueMicrotask()` 立即分发最新 frame packet

对应文档：

- `docs/requirements/2026-04-03-scanner-yolo-realtime-structural-only.md`
- `docs/plans/2026-04-03-scanner-yolo-realtime-structural-only-execution-plan.md`
- `docs/zh/scanner/2026-04-03-scanner-yolo-realtime-structural-summary.md`

## 验证与脚本

### 测试脚本

已新增：

- `scripts/test-scanner-yolo-realtime-structural.ps1`

它会执行：

- `pnpm exec tsc --noEmit --pretty false`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo build --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-benchmark`
- `.\\src-tauri\\target\\debug\\scanner-native-yolo-benchmark.exe .github/images/skidhw-thumbnail-new.png --iterations 5`
- `.\\src-tauri\\target\\debug\\scanner-native-yolo-smoke.exe .github/images/skidhw-thumbnail-new.png --raw-rgba --max-width 320 --max-height 180`

### 最近一次实际运行结果

最近一次完整执行：

```powershell
pwsh -NoProfile -File .\scripts\test-scanner-yolo-realtime-structural.ps1
```

最近一次 benchmark 结果：

- `encoded-fullres`: `463.58722ms`
- `raw-rgba-bounded-320x180-cached`: `406.69758ms`
- `raw-rgba-bounded-320x180-reset-each-run`: `19946.8126ms`

最重要的结论是：

- **cache 复用** 是真实收益点
- 同参数下，`cached` 相比 `reset-each-run` 大约有 **49x~60x** 量级差距
- 这证明收益来自结构，而不是调参

## 运行时 / GPG / 环境坑点

### 1. GPG 提交流程

- 当前已经成功做出带 `gpgsig` 的提交 `fcc9855`
- 但本机 `git show --show-signature` 时，Git for Windows 的 `gpg.exe` 包装层会打印
  `couldn't create signal pipe, Win32 error 5`
- 这个噪声 **不代表提交没签上**
- 我已经直接看过 commit object，里面有 `gpgsig` 块

### 2. docs / outputs / scripts 默认被 `.gitignore` 忽略

重要：

- `docs/*`
- `docs/zh/scanner/*`
- `outputs`
- `scripts/*`

这些路径默认被忽略。

所以本次提交时是通过 **`git add -f`** 强制纳入的。

如果继任者要继续新增同类文档 / receipt / 脚本，记得：

- 正常创建文件没问题
- 但如果要进 git，需要 `git add -f`

### 3. `git status --short` 曾经出现假脏状态

本次整理剩余未提交项时，以下文件曾显示 `M`：

- `.gitignore`
- `src-tauri/src/adb_plugin.rs`
- `src-tauri/src/bin/scanner-real-benchmark.rs`
- `src-tauri/src/lib.rs`

但逐个核对后确认：

- 工作区 hash 与 `HEAD` 一致
- `git diff` 为空
- 最后通过索引状态恢复，`git status --short` 已清空

如果下次再遇到这种现象，先核实 hash 和 diff，不要误以为是实际代码改动。

## 当前最值得继续的方向

如果要继续压榨实时链路，优先级建议如下：

### P0

排查并 benchmark：

- `src/lib/scanner/frame-source.ts`
- `src/lib/scanner/frame-codec.ts`

原因：

- 当前用户一开始报的是 **`ipc wait` 1000ms+**
- 这个指标来自 preview streaming path 的 `lastIpcMs`
- 不是 `scanner_detect.rs` 自身的推理耗时

也就是说，下一轮最应该继续砍的是：

- preview packet decode
- `decodeFramePacketToRgba()` 主线程成本
- `new ImageData(...)` / `putImageData(...)` / portrait 旋转路径

### P1

给 preview decode 链路补单独 benchmark：

- 不要只测 native yolo detect
- 要把 preview stream packet decode 和 canvas upload 单独拆出来测

### P2

如果再动 commit，继续遵守：

- 不把无关脏文件带进 commit
- 文档 / outputs / scripts 如需提交要 `git add -f`
- 用户如果要求 GPG，就不要 silently fallback 到 unsigned

## 给继任者的建议

1. 先读：
   - `docs/zh/scanner/2026-04-02-scanner-yolo-performance-runtime-summary.md`
   - `docs/zh/scanner/2026-04-03-scanner-yolo-realtime-structural-summary.md`
2. 再跑：
   - `pwsh -NoProfile -File .\scripts\test-scanner-yolo-realtime-structural.ps1`
3. 再决定是否继续进入：
   - `frame-source.ts`
   - `frame-codec.ts`
   的主线程 decode / upload 优化

不要回到“继续调 `320x180` / `120ms` / 阈值”的路线，用户已经明确否决了那种优化方式。
