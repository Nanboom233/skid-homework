# 2026-04-02 扫描器 YOLO 可选后端需求冻结（修订版）

## 背景

当前扫描器的文档边界检测仍由前端链路负责：

- `src/components/scanner/ScannerView.tsx` 中的 `detectDocumentContourWithFallback()` 负责实时检测；
- 优先走 `src/lib/scanner/cv-worker-client.ts` + `src/lib/scanner/cv-detection.worker.ts`；
- 失败时回退到 `src/lib/scanner/document-detector.ts` 的主线程 OpenCV 检测；
- 下游裁切、透视矫正、手动重裁统一消费 `Point[] | null`，理想值为 4 个有序角点。

同时，仓库已经存在桌面原生图像后处理链路：

- 前端桥接：`src/lib/tauri/scanner.ts`
- Rust 命令：`src-tauri/src/scanner_postprocess.rs`
- Tauri 注册：`src-tauri/src/lib.rs`

用户已明确修正方案方向：

1. **YOLO 不再走浏览器侧 worker / ONNX Runtime Web；**
2. **YOLO 改为桌面版特供能力；**
3. **实现位置必须在 Rust / Tauri 原生侧；**
4. **Windows 优先使用 DirectML；**
5. **Linux 优先使用 TensorRT，其次 CUDA；**
6. **当前没有可直接使用的本地 YOLO 模型。**

## 目标

在不破坏现有 OpenCV 前端扫描链路的前提下，为 **Tauri 桌面版** 增加一个 **Rust 原生 YOLO 检测后端**，使其作为扫描器的可选检测引擎接入当前链路，并满足以下平台策略：

- Windows：`DirectML -> CPU fallback`
- Linux（NVIDIA 环境）：`TensorRT -> CUDA -> CPU fallback`

同时，针对“文档/书本大致平放，但局部可能斜侧、靠书脊区域文字需要展平”的真实目标，后续方案必须按 **两阶段链路** 设计：

1. **阶段一：定位与粗裁切**
   - 找到页面 / 书页区域；
   - 输出稳定粗 ROI / 粗角点；
   - 优先高召回与抗背景干扰。
2. **阶段二：几何矫正与识别复用**
   - 对阶段一的 ROI 做精细透视校正；
   - 对轻度书脊侧弯曲文字做展平；
   - 输出可复用给 OCR / CV 识别的标准化图像。

## 关键结论（本轮规划冻结）

### 1. 运行时路线

- 最可行的 Rust 推理路线是 **ONNX Runtime + Rust 绑定（优先 `ort` crate）**；
- `tract`、`wonnx` 等替代方案不适合本任务，因为它们无法实际提供本任务要求的 DirectML / TensorRT / CUDA execution providers；
- 由于 Windows DirectML 不适合依赖简单“在线下载预编译 ORT”策略，本项目应采用 **平台分发的原生 ORT 动态库打包方案**，而不是浏览器或纯前端方案。

### 2. 平台能力与约束

- **DirectML** 官方支持 Windows GPU 推理，但已进入 sustained engineering；对本项目来说仍可用，但需记录其长期演进风险；
- **TensorRT / CUDA** 在 Linux 上可行，但依赖目标机存在匹配的 NVIDIA 驱动、CUDA、cuDNN、TensorRT 运行时；
- 因此 Linux 方案不能假设“零前置环境”，必须保留 CPU 回退并暴露明确可诊断状态。

### 3. 模型现实

- 没有证据表明当前存在一个“广泛采用、可直接落地、面向通用扫描器页边界检测”的现成 YOLO 公共模型；
- 因而本项目不能把“下载现成通用模型”当作主计划；
- 更现实的模型路线应在：
  - **小型 ONNX 模型 + 自行微调**
  - **低成本合成数据训练**
  - **MIDV-500 / MIDV-2019 / SmartDoc 等真实数据补强**
  三者之间组合。

### 4. 数据集判断

- **适合页边界 / 四角点任务**：
  - MIDV-500
  - MIDV-2019
  - SmartDoc（次优、补充）
- **不适合直接训练扫描器外框检测**：
  - DocLayNet
  - PubLayNet

原因是后两者主要面向版面分析、内部布局框，不是相机拍摄场景下的外边界四角点。

## 交付范围

本需求冻结对应后续实现应覆盖：

1. 桌面端原生 YOLO 检测后端抽象；
2. Rust/Tauri 原生检测命令；
3. ONNX Runtime 平台化 provider 选择与诊断；
4. YOLO 输出映射为当前扫描链路的 `Point[] | null`；
5. 桌面端 UI / 设置中的检测后端选择；
6. 模型不可用、provider 不可用时的显式回退与提示；
7. 文档、验证脚本与构建验证证据。

## 强约束

### 功能约束

- OpenCV 现有链路必须保留；
- Web / 非 Tauri 环境不得依赖 YOLO；
- 桌面 YOLO 必须兼容现有四角点契约；
- 检测失败时不得阻断扫描器主流程。

### 平台约束

- Windows 桌面版：
  - 首选 DirectML；
  - 不依赖 NVIDIA 专属栈；
  - 保留 CPU 回退。
- Linux 桌面版：
  - 首选 TensorRT；
  - 次选 CUDA；
  - 若目标机缺少 NVIDIA 运行时则必须回退 CPU。

### 工程约束

- 推理 session 必须缓存，不能每帧重建；
- 不能把大模型路径硬编码在前端；
- 模型与原生依赖必须通过 Tauri 打包资源管理；
- 必须提供 provider / model / fallback 的调试信息。

## 推荐实现方向

### 推荐主线

**桌面原生 YOLO = Rust + ONNX Runtime + 平台化 EP 策略**

- Windows：`DirectML -> CPU`
- Linux：`TensorRT -> CUDA -> CPU`

### 推荐模型路线

由于当前无模型资产，推荐按优先级采用：

1. **优先：YOLO Pose（4 keypoints）+ 合成数据生成 + 小模型微调**
   - 直接输出 `topLeft / topRight / bottomRight / bottomLeft`
   - 自动生成 page-on-background 样本
   - 零人工标注成本
   - 与当前四角点契约最匹配
2. **补强：MIDV-500 / MIDV-2019**
   - 真实拍摄条件四角点样本
   - 可直接转换为 4-keypoint 标签
   - 用于降低纯合成域偏移
3. **补充验证：SmartDoc**
   - 用于复杂光照与旧移动端场景

### 两阶段职责修订

- **YOLO 负责阶段一：鲁棒定位 / 粗裁切**
- **CV / 几何后处理负责阶段二：精细矫正 / 文字展平 / OCR 复用**

这意味着：

- 阶段一模型不必独自承担“最终 OCR-ready 几何质量”；
- 阶段二可以复用并扩展现有 `scanner_postprocess.rs` 的透视与增强能力；
- 对书本内侧轻度弯曲文字，应在阶段二增加局部展平，而不是强迫阶段一检测器直接输出最终完美页面。

### 模型输出建议

当前链路要求的是“真实透视四边形角点”，因此模型任务优先级修订为：

1. **YOLO Pose / 4-keypoint 回归（推荐）**
2. **Segmentation + quad fitting（高几何保真备选）**
3. **OBB（不建议作为主方案）**
4. **普通 detect（不建议）**

补充说明：

- **Pose** 最适合当前工程，因为公开数据（MIDV）本身就接近“四角点标注”；
- **Segmentation** 的几何匹配度高，但需要 mask / contour 后处理，训练标注成本也更高；
- **OBB** 虽然也输出 4 点，但本质是“旋转矩形”，而扫描器页面在透视下通常是一般四边形，不一定是矩形，因此存在几何失配；
- **普通 detect** 只给水平框，和当前透视裁切目标不匹配。

## 验收标准

后续实现完成时，应满足：

1. 桌面版扫描器允许切换 OpenCV / Native YOLO；
2. Windows 能正确探测并优先尝试 DirectML；
3. Linux 能正确探测并优先尝试 TensorRT，再尝试 CUDA；
4. 任一 provider 不可用时系统能继续运行；
5. 原生检测返回的角点能驱动现有裁切流程；
6. 无模型或依赖缺失时 UI / 调试面板给出可解释状态；
7. Web 版本行为无功能回归。

## 非目标

- 本轮不实现浏览器侧 ORT Web / WebGPU YOLO；
- 不承诺“无需任何 NVIDIA 依赖即可在 Linux GPU 上运行 TensorRT/CUDA”；
- 不把 DocLayNet / PubLayNet 误用为扫描器边界检测主数据集；
- 不在本轮承诺完整训练流水线自动化；
- 不替换现有后处理、增强、透视裁切主架构。

## 参考资料

- ONNX Runtime Execution Providers: https://onnxruntime.ai/docs/execution-providers/
- Ultralytics Pose task: https://docs.ultralytics.com/tasks/pose/
- Ultralytics Pose datasets: https://docs.ultralytics.com/datasets/pose/
- DirectML EP: https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- TensorRT EP: https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- CUDA EP: https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
- Rust `ort` crate: https://github.com/open-runtime/ort-rust
- MIDV-500: https://arxiv.org/abs/1507.00390
- MIDV-2019: https://arxiv.org/pdf/1910.03184
- DocLayNet: https://arxiv.org/abs/2206.01062
- PubLayNet: https://arxiv.org/abs/1908.03213
