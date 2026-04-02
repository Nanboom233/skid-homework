# Scanner YOLO Optional Backend Execution Plan（桌面 Rust 修订版）

## Goal

将原本前端侧假设的 YOLO 可选后端，重构为 **Tauri 桌面原生 YOLO 检测链路**：

- Windows：DirectML
- Linux：TensorRT / CUDA
- 统一输出四角点给现有扫描器
- 保留 OpenCV 现有路径

## 总体策略

本计划改为 **两条并行主线**：

1. **运行时主线**：先做 Rust 原生推理骨架、provider 探测、资源打包与回退；
2. **模型主线**：并行准备低成本模型路线（YOLO Pose + synthetic + MIDV 微调），因为当前没有可直接使用的模型。

在模型未就绪前，优先完成：

- 运行时位点
- 资源位点
- UI / debug / fallback 语义

## Repo-grounded 事实

- 实时检测入口：`src/components/scanner/ScannerView.tsx`
- 当前检测函数：`detectDocumentContourWithFallback()`
- 已有原生桥接：`src/lib/tauri/scanner.ts`
- 已有原生命令：`src-tauri/src/scanner_postprocess.rs`
- Tauri 注册点：`src-tauri/src/lib.rs`
- Tauri 已启用 `src-tauri/resources/*` 打包：`src-tauri/tauri.conf.json`

这意味着新方案应优先沿用 **既有 Tauri 原生命令模式**，而不是回到浏览器 worker。

## 2026-04-02 实施进度补记

Wave 1 的最小骨架已落地：

- 已新增 `src-tauri/src/scanner_detect.rs`
- 已注册并实现：
  - `tauri_scanner_probe_yolo`
  - `tauri_scanner_detect_document`
- 已新增 CLI probe：
  - `src-tauri/src/bin/scanner-native-yolo-probe.rs`
- 已新增 TS bridge：
  - `src/lib/tauri/scanner-detect.ts`
- 已新增测试脚本：
  - `scripts/test-scanner-native-yolo-skeleton.ps1`

当前状态不是“推理完成”，而是：

- 工程已恢复可编译
- 资源位点已固定
- provider / model 缺失会被诚实报告
- ORT crate 已真实接入
- runtime 动态库加载与 session 初始化代码路径已完成
- 当前阻塞点变成“runtime 资源与模型缺失”

`2026-04-02 resume` 之后，实施状态再向前推进了一步：

- Scanner UI 已完成：
  - backend selector
  - active backend / provider / model debug
  - strict mode
  - settings persistence
- Rust runtime 已补上 DirectML 官方 session 约束：
  - sequential execution
  - disabled memory pattern
- 二次验证已确认：
  - `cargo check --manifest-path src-tauri/Cargo.toml`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe`
  - `pnpm exec tsc --noEmit --pretty false`

当前最准确的项目状态应表述为：

> `native-yolo` 作为桌面可选 backend 已经真实接上，  
> 但当前激活模型仍是 public baseline，而不是最终 YOLO pose 主模型。

额外修正两点：

1. 公开 YOLO scanner 模型并非完全没有，但大多是 **bbox 粗定位**，可做 baseline，不应替代四角点主路线；
2. Linux 侧更适合把 CUDA / TensorRT 视为系统前置依赖，而不是尝试把完整 NVIDIA 运行时随仓库打包。

新增一条工程判断：

3. 由于目前没有权威确认可直接落地的公共 4-corner YOLO pose 成品模型，Wave 6 之前不应承诺“只差把模型文件放进去就完成”。

---

## Wave 0：冻结模型与运行时边界

### Outcome

明确：

- YOLO 仅桌面可用；
- 推理在 Rust；
- Windows / Linux provider 顺序；
- 模型任务优先采用 **YOLO Pose（4 keypoints）**；
- segmentation + quad fitting 作为高几何保真备选；
- OBB 不作为主路线；
- 数据集主路线为 synthetic + MIDV-500 / MIDV-2019。

### Deliverables

- `docs/requirements/2026-04-02-scanner-yolo-optional-backend.md`
- `docs/plans/2026-04-02-scanner-yolo-optional-backend-design.md`
- `docs/zh/scanner/2026-04-02-scanner-yolo-optional-backend-summary.md`

---

## Wave 1：Rust 原生检测骨架

### Files

- Create: `src-tauri/src/scanner_detect.rs`
- Create: `src-tauri/src/scanner_detect_runtime.rs`（可选）
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

### Task outcome

Tauri 桌面端可以：

1. 探测原生 YOLO 能力；
2. 初始化 ORT session；
3. 以 provider 顺序尝试加载；
4. 返回可解释的 availability 结果。

### Step 1: 引入 Rust 推理依赖

目标：

- 引入 `ort` crate 或等效 ORT Rust binding；
- 为后续 provider 编排留出 Cargo feature / build 策略。

补充约束：

- Windows 建议优先走 `load-dynamic` + 本地资源打包 `onnxruntime.dll` / `DirectML.dll`
- Linux 不建议把完整 CUDA / TensorRT 运行时直接塞进仓库资源目录
- Linux 应改为：
  - 只管理我们自己的模型与配置
  - 对系统 CUDA / TensorRT 做 probe / prerequisite check

### Step 2: 建立原生命令

新增：

- `tauri_scanner_probe_yolo`
- `tauri_scanner_detect_document`

### Step 3: 建立 session manager

要求：

- 进程级缓存；
- 不得每帧重建；
- 错误状态可缓存。

---

## Wave 2：平台化 provider 策略

### Files

- Modify: `src-tauri/src/scanner_detect.rs`
- Modify: `src-tauri/src/scanner_detect_runtime.rs`
- Add/Document: `src-tauri/resources/onnxruntime/...`

### Task outcome

平台化 provider 顺序固定为：

- Windows：`DirectML -> CPU`
- Linux：`TensorRT -> CUDA -> CPU`

### Step 1: Windows provider 实现

要求：

- 先尝试 DirectML；
- 明确记录：
  - provider 是否注册成功
  - 模型是否可加载
  - 失败原因

### Step 2: Linux provider 实现

要求：

- 先尝试 TensorRT；
- 失败则尝试 CUDA；
- 再失败则落 CPU。

### Step 3: 失败语义统一

统一错误分类：

- `model_missing`
- `runtime_missing`
- `provider_unavailable`
- `session_init_failed`
- `inference_failed`

---

## Wave 3：模型资源与配置位

### Files

- Add: `src-tauri/resources/models/document-boundary.onnx`（占位或文档说明）
- Add: `src-tauri/resources/scanner-yolo-config.json`
- Add: `src-tauri/resources/models/README.md`

### Task outcome

模型与运行时依赖的放置位置、命名规则、配置格式固定。

### Step 1: 固定模型路径

建议：

```text
src-tauri/resources/models/document-boundary.onnx
```

### Step 2: 固定模型配置

例如：

```json
{
  "task": "pose",
  "inputWidth": 640,
  "inputHeight": 640,
  "scoreThreshold": 0.25,
  "iouThreshold": 0.45
}
```

### Step 3: 文档化模型缺失状态

即使模型未提交仓库，也必须让 probe 命令能明确报出：

```text
native yolo configured but model missing
```

---

## Wave 4：前端桥接与设置接入

### Files

- Create: `src/lib/tauri/scanner-detect.ts`
- Modify: `src/store/settings-store.ts`
- Modify: `src/components/scanner/ScannerControls.tsx`
- Modify: `public/locales/zh/commons.json`
- Modify: `public/locales/en/commons.json`

### Task outcome

桌面端可选择：

- OpenCV
- Native YOLO

且能看到当前 native yolo 是否可用。

### Step 1: TS 桥接

桥接层负责：

- 调 `probe`
- 调 `detect`
- 归一化 Rust 返回

### Step 2: 设置持久化

新增设置建议：

- `scannerDetectionBackend: "opencv" | "native-yolo"`
- `scannerNativeYoloStrictMode`
- `scannerNativeYoloLinuxProviderPreference`

### Step 3: Desktop-only UI

要求：

- 非 Tauri 环境不展示或置灰 YOLO；
- 桌面环境展示 availability。

---

## Wave 5：ScannerView 实时链路接入

### Files

- Modify: `src/components/scanner/ScannerView.tsx`
- Modify: `src/store/scanner-store.ts`
- Modify: `src/components/scanner/ScannerDebugPanel.tsx`

### Task outcome

检测结果来源可切换为：

- 现有 OpenCV
- 新的 Native YOLO

但 tracker / stable-hold / auto-capture 逻辑保持不变。

### Step 1: 抽象检测分发层

将当前：

```ts
detectDocumentContourWithFallback()
```

扩展为后端分发：

- `opencv`
- `native-yolo`

### Step 2: 控制 IPC 成本

要求：

- 前端先下采样；
- 限制 native detect 调用频率；
- 拍照后重检可用更高质量输入。

### Step 3: Debug 面板

显示：

- active backend
- active provider
- model state
- inference time
- fallback reason

### Step 4: 明确阶段二 ownership

本 wave 接入时必须同步明确：

- **阶段一** = native yolo 定位 / 粗裁切
- **阶段二** = CV 几何精修 / 透视 / 轻度展平 / OCR 复用

不要把“最终 OCR-ready 几何质量”全部压给阶段一检测模型。

---

## Wave 6：模型主线（并行，不阻塞骨架）

### Outcome

拿到可部署的 ONNX 模型。

### Path A（推荐）

```text
synthetic page compositing
-> convert to 4-keypoint pose labels
-> fine-tune YOLO Pose small model
-> export ONNX
-> desktop inference validation
```

### Path B

```text
MIDV-500 / MIDV-2019 conversion
-> 4-keypoint pose labels
-> mixed fine-tune
-> SmartDoc validation
```

### Path C（仅备选）

```text
public model reuse
```

仅当实际拿到：

- 明确 license
- 明确权重
- 明确任务契约
- 明确 ONNX 导出可行

才纳入。

### 模型主线额外说明

- **首选模型族**：`yolo*n-pose`
- **首选标签语义**：
  - keypoint 0 = top-left
  - keypoint 1 = top-right
  - keypoint 2 = bottom-right
  - keypoint 3 = bottom-left
- **不建议主用 OBB**：因为它描述的是旋转矩形，不是一般透视四边形

---

## Wave 7：验证与分发

### 当代码改动开始后，必须执行

#### TypeScript

```powershell
pnpm exec tsc --noEmit --pretty false
```

#### ESLint

```powershell
pnpm exec eslint src/components/scanner/ScannerView.tsx src/components/scanner/ScannerControls.tsx src/components/scanner/ScannerDebugPanel.tsx src/store/settings-store.ts src/store/scanner-store.ts src/lib/tauri src/lib/scanner
```

#### Frontend build

```powershell
pnpm build
```

#### Rust

```powershell
cargo check --manifest-path src-tauri/Cargo.toml
```

#### Tauri desktop build

```powershell
pnpm tauri:build
```

### 额外验证

#### Windows

- DirectML 可注册
- 模型可加载
- 能返回四角点
- 失败时回退 CPU 或 OpenCV

#### Linux

- TensorRT 可注册时优先使用
- 无 TensorRT 时尝试 CUDA
- 二者都失败时 CPU / OpenCV 仍可工作

---

## 优先级建议

### P0

- 修订文档
- Rust 运行时骨架
- provider probe
- 资源位点

### P1

- 前端设置
- 调试面板
- 实时检测接入

### P2

- 模型训练与精度迭代
- Linux 打包与安装器优化

---

## 当前建议的下一步

在真正开始代码实现前，建议先确认两件事：

1. **模型任务是否按推荐方案固定为 YOLO Pose（4 keypoints），并仅把 segmentation 作为备选；**
2. **Linux 发行为“仅支持已装 NVIDIA 栈的机器”，还是要把 GPU 能力视为增强项并默认 OpenCV。**

在此之前，不建议继续沿用旧的浏览器 worker 计划。
