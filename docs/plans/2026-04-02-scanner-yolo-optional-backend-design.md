# 扫描器 YOLO 可选后端设计（桌面 Rust 修订版）

## 0. 2026-04-02 资料复核修订

本设计在 `2026-04-02` 当天做了二次收束，复核来源为：

- DirectML 官方文档
  - https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- TensorRT 官方文档
  - https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- CUDA 官方文档
  - https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
- Ultralytics Pose / Export 官方文档
  - https://docs.ultralytics.com/tasks/pose/
  - https://docs.ultralytics.com/modes/export/
- MIDV-500 主源
  - https://smartengines.com/wp-content/uploads/2020/04/datasets-of-id-documents-midv-500.pdf
  - https://arxiv.org/abs/1807.05786

这次复核后，需要把设计前提明确成下面四条：

1. Windows DirectML 不是“随便挂一个 provider 就完”，而是有明确 session 约束：
   - sequential execution
   - disabled memory pattern
   - same-session `Run()` 串行
2. Linux 路线应固定写成：
   - `TensorRT -> CUDA -> CPU`
3. 当前没有可权威确认、可直接替代本项目契约的“公共 4-corner YOLO pose 成品模型”
4. 因此当前交付必须接受两阶段事实：
   - 先交付 public baseline
   - 后补训练好的 YOLO Pose 4-corner 主模型

## 1. 设计结论

原先“浏览器 worker + ONNX Runtime Web”的方向已废弃。  
本项目的修订设计为：

- **YOLO 仅面向 Tauri 桌面版**
- **推理由 Rust 原生侧执行**
- **统一模型格式为 ONNX**
- **统一运行时为 ONNX Runtime**
- **Windows 使用 DirectML**
- **Linux 使用 TensorRT / CUDA**
- **前端仍消费 `Point[] | null`**

但从真实目标重新推导后，YOLO 的职责也需要收敛：

> **YOLO 不应被设计成“一步直接产出最终 OCR-ready 页面”的万能模块。**

更合理的边界是：

- **阶段一：YOLO 做定位与粗裁切**
- **阶段二：CV / 几何后处理做精细透视校正、轻度曲面展平与 OCR 复用**

## 1.2 当前实现状态的设计定位

截至 `2026-04-02`，仓库里已经存在可真实运行的桌面原生 stage-1 backend，但它分成两个层次：

### 当前已交付能力

- Rust + ONNX Runtime
- Windows DirectML runtime / session / probe
- `native-yolo` 已接进 Scanner UI，可切换、可 debug、可 strict mode
- public baseline `docaligner-fastvit-sa24` 已能真实返回四角点

### 尚未交付能力

- 真实 `document-boundary-yolo-pose-4pt` 模型文件
- YOLO pose 输出解码器
- Linux TensorRT / CUDA 真机部署验收

因此“当前 runtime 已接入”和“最终主模型尚未完成”必须被同时表达，不能再把 public baseline 包装成已经拥有最终 YOLO。

## 1.1 两阶段扫描链路

### 阶段一：Detection / Crop

职责：

- 从复杂背景中找到文档或书页；
- 产出粗 ROI、粗角点或粗 mask；
- 为后续后处理缩小搜索空间。

### 阶段二：Rectify / Flatten / OCR Reuse

职责：

- 精细角点修正；
- 平面透视矫正；
- 轻度书脊侧弯曲文字展平；
- 图像增强；
- 输出给 OCR / CV 识别复用。

这意味着阶段二必须是**独立可复用模块**，不能被隐藏在检测模型内部。

## 2. 现有仓库中的可接入点

### 2.1 前端检测入口

`src/components/scanner/ScannerView.tsx`

当前这里通过 `detectDocumentContourWithFallback()` 完成检测：

- 优先前端 CV worker；
- 失败后回到主线程 OpenCV；
- 实时预览和拍照后重检都依赖这一路径。

### 2.2 已存在的 Tauri 原生桥

- 前端桥：`src/lib/tauri/scanner.ts`
- Rust 实现：`src-tauri/src/scanner_postprocess.rs`
- 注册入口：`src-tauri/src/lib.rs`

这说明仓库已经具备：

1. JS -> Rust invoke 通路；
2. 原生侧重任务通过 `spawn_blocking` 执行的模式；
3. 原生返回结构化元数据 + 二进制 payload 的能力。

因此，YOLO 最自然的落点不是再新造浏览器 worker，而是：

1. **仿照 scanner_postprocess 增加 scanner_detect 原生命令**，承担阶段一；
2. **扩展现有 scanner_postprocess**，承担阶段二。

## 3. 推荐运行时栈

## 3.1 推理引擎

推荐：**ONNX Runtime + Rust `ort` crate**

原因：

- 当前最现实的 Rust ONNX 推理绑定；
- 能接 ONNX Runtime execution providers；
- 能覆盖本项目需要的 DirectML / TensorRT / CUDA；
- 适合做桌面版平台差异化加载。

### 设计注意

`ort` 的“download strategy”不适合作为本项目最终方案，因为：

- Windows 的 DirectML 不应依赖简单下载式预编译包；
- 本项目需要 **按平台自行控制 ORT 动态库与 provider 依赖**；
- 因此应优先采用 **system / dynamic library strategy**，让 Tauri 打包资源显式携带对应平台的 ORT 动态库。

## 3.2 不推荐路线

- `tract`：偏 CPU / 纯 Rust 路线，不满足 DirectML / TensorRT / CUDA 要求；
- `wonnx` / WebGPU 路线：不符合桌面原生与平台 EP 目标；
- 浏览器侧 ORT Web：已被用户明确否决。

## 4. 平台执行策略

## 4.1 Windows

优先链：

```text
DirectML -> CPU
```

理由：

- DirectML 对 Windows 多 GPU 厂商兼容性更好；
- 不强依赖 NVIDIA 生态；
- 更符合“桌面特供但不限定显卡品牌”的产品目标。

风险：

- DirectML 已进入 sustained engineering；
- 需要记录长期演进风险；
- 但在当前阶段仍是最贴合需求的 Windows 方案。

## 4.2 Linux

优先链：

```text
TensorRT -> CUDA -> CPU
```

理由：

- 若目标机具备完整 NVIDIA 栈，TensorRT 可提供最佳吞吐；
- CUDA 是更宽松的第二选择；
- CPU 回退保证功能可用性。

限制：

- TensorRT / CUDA 依赖目标机已安装匹配版本的：
  - NVIDIA Driver
  - CUDA Runtime
  - cuDNN
  - TensorRT（TensorRT 路径）
- 这些依赖不应承诺被项目“完全静态打包替代”。

## 5. 模型与任务类型设计

## 5.1 当前业务契约

扫描器下游需要：

```ts
Point[] | null
```

且理想值是：

```ts
[topLeft, topRight, bottomRight, bottomLeft]
```

因此模型并不是“只要能看到文档就行”，而是必须能稳定输出 **可排序的四边界信息**。

## 5.2 模型任务优先级（修订）

### 一级推荐：YOLO Pose / 4-keypoint

这是当前最推荐的主路线。

原因：

1. Ultralytics 官方支持 pose / keypoint 任务；
2. 官方支持自定义 keypoint 数量；
3. ONNX export 可行；
4. 对本项目来说可以把 keypoints 直接定义为：
   - top-left
   - top-right
   - bottom-right
   - bottom-left
5. MIDV-500 / MIDV-2019 的标签天然更接近四角点，而不是 mask。

这意味着模型输出可以直接对接当前扫描器契约，而不再需要从旋转框或水平框中反推真实角点。

### 二级推荐：Segmentation + Quad Fitting

这是**几何保真度最高的备选路线**。

流程为：

1. 输出文档 mask；
2. 提取轮廓；
3. 做 polygon / quad 拟合；
4. 统一排序为当前角点顺序。

优点：

- 对强透视形变更自然；
- 不依赖“矩形”假设。

缺点：

- 训练标注更偏向 mask；
- 后处理复杂度更高；
- 初版工程成本高于 pose。

### 不推荐：OBB 作为主路线

OBB 最大问题不是“没有 4 点”，而是**它的 4 点描述的是旋转矩形**。  
而相机下的纸张通常是**一般透视四边形**，不一定保持矩形性质。

因此：

- OBB 的 4 点可能只是“最佳旋转包围框”的顶点；
- 它们不一定等于真实页面四角；
- 在强透视、梯形畸变下误差会更系统化。

所以 OBB 可以作为实验性备选，但**不应作为主模型路线**。

### 不推荐：普通 detect

普通 detect 只给水平框，无法直接驱动透视裁切，不适合作为最终检测模型。

## 5.3 模型现实结论

当前没有足够证据证明存在“可直接拿来上线”的通用扫描器 YOLO 公共模型。  
因此设计必须接受一个前提：

> **模型获取与模型接入要拆开做。**

也就是说：

- 运行时骨架可以先规划；
- 但最终准确率不能建立在“下载一个现成通用模型”上。

进一步地，基于本轮检索结果，更准确的说法是：

- 公开的 document detection / scanner 模型不是完全没有
- 但公开模型大多给的是：
  - bbox
  - bbox + 几何后处理
  - 或非 YOLO 的四角点 baseline
- 当前**没有找到一个权威、现成、公共可下载、且直接输出 4 个有序文档角点的 YOLO pose 模型**

所以当前公共 baseline 的设计定位应是：

> 可上线的 stage-1 过渡模型，而不是“已经完成 YOLO 主线”的伪装。

## 6. 模型获取策略

## 6.1 推荐主路线

```text
synthetic page compositing
-> YOLO Pose（4 keypoints）小模型微调
-> MIDV-500 / MIDV-2019 补强
-> SmartDoc 验证
```

### 原因

1. 当前无本地模型；
2. 公共现成模型不可靠；
3. 纯人工标注成本高；
4. 合成数据可自动产生四角点标签，成本最低；
5. MIDV 标签能较低成本转换为 4-keypoint pose 标签。

### 6.1.1 为什么 MIDV-500 现在是主源

基于 Smart Engines 官方白皮书可确认：

- MIDV-500 提供 500 段视频
- 拆帧后约 15,000 帧
- 每帧包含 `quad` 四角点 JSON
- 场景覆盖：
  - table
  - keyboard
  - hand
  - partial
  - clutter

这意味着 MIDV-500 不是泛泛的“文档数据集”，而是可以较低成本转换成：

```text
class + bbox + 4 keypoints
```

从而直接对接 Ultralytics pose 训练格式。

### 6.1.2 极低成本训练版本的现实做法

如果目标是尽快拿到第一版可部署 YOLO，而不是一次追求最高指标，建议把成本压到下面这条路径：

1. synthetic 样本承担大部分姿态 / 背景覆盖
2. MIDV-500 / MIDV-2019 负责真实域补强
3. 模型先从小规格 pose 开始：
   - `yolo*n-pose`
4. 输入尺寸先固定：
   - `640`
5. 导出先用：
   - ONNX
   - Linux 再视情况补 TensorRT engine

这条路线的主要成本不是买数据，而是：

- 数据转换脚本
- synthetic 生成脚本
- 训练 / 验证 / 导出

## 6.2 数据集角色

### 主训练/补强

- MIDV-500
- MIDV-2019

### 辅助验证

- SmartDoc

### 不纳入主路线

- DocLayNet
- PubLayNet

原因：它们更适合版面分析，不适合相机扫描场景下的外边界检测。

## 7. Tauri 侧架构设计

## 7.1 新增 Rust 模块

建议新增：

- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/scanner_detect_runtime.rs`（可选拆分）

职责：

1. provider 选择；
2. session 初始化与缓存；
3. 图像预处理；
4. ONNX 推理；
5. 输出后处理为有序四角点；
6. 返回诊断信息。

### 7.1.1 当前 runtime contract 需要额外固定的限制

Windows DirectML 路径必须被视为有官方约束的专用 session：

- `with_parallel_execution(false)`
- `with_memory_pattern(false)`
- 同一 session 的 `Run()` 必须串行

后续即使继续重构 runtime，也不能把这三个条件在抽象层里弄丢。

## 7.2 新增命令接口

建议提供两个原生命令：

### `tauri_scanner_probe_yolo`

用于启动时或设置切换时探测：

- 当前是否在 Tauri；
- 模型文件是否存在；
- ORT 动态库是否可加载；
- DirectML / TensorRT / CUDA 是否可注册；
- 最终将使用的 provider；
- 失败原因。

### `tauri_scanner_detect_document`

用于实际检测，输入：

- RGBA bytes 或下采样后的 image bytes；
- width / height；
- 可选检测参数（阈值、输入尺寸、provider 偏好）。

输出：

- `points: Point[] | null`
- `processingMs`
- `provider`
- `modelState`
- `fallbackReason`

## 7.3 Session 生命周期

必须使用 **进程级缓存 session**。

推荐：

- `OnceLock` / `Mutex` / `RwLock`
- 或 `tauri::State<...>`

目标：

- 模型只初始化一次；
- 切换页面不重复加载；
- provider 初始化失败结果可缓存，避免每帧重复报错。

## 8. 前端桥接设计

## 8.1 新增桥接文件

建议新增：

- `src/lib/tauri/scanner-detect.ts`

职责：

1. 调用 `tauri_scanner_probe_yolo`
2. 调用 `tauri_scanner_detect_document`
3. 对原生错误做 TS 侧统一归一化
4. 暴露 provider / error / latency 元数据

## 8.2 ScannerView 接入策略

`ScannerView.tsx` 当前实时循环已经有稳定的检测调度逻辑，因此推荐：

### 桌面 + Native YOLO 选中时

- 前端先缩小输入尺寸；
- 以较低频率调用原生检测；
- 保持现有 tracker / stable hold / auto capture 逻辑不动；
- 仅替换“检测结果来源”。

### Web 或 Native YOLO 不可用时

- 继续走现有 OpenCV 路径。

## 8.4 阶段二后处理策略

### 场景 A：普通平面文档

当页面基本平整时，阶段二只需要：

1. 精细角点修正；
2. perspective transform；
3. enhance / denoise；
4. 输出 OCR-ready 图像。

这部分可在现有 `scanner_postprocess.rs` 基础上增强。

### 场景 B：轻度书页 / 书脊侧弯曲

当问题主要来自书脊附近文字轻微弯曲时，仅靠四点透视变换通常不够。  
此时阶段二需要新增：

1. text-line / edge-based curvature estimation
2. 局部网格或位移场展平
3. 再做增强与 OCR 输出

也就是说：

- **阶段一仍然可以复用同一个 YOLO 检测器**
- **真正的“展平文字”责任属于阶段二**

## 8.3 IPC 成本控制

原生推理每帧都走 JS <-> Rust IPC 会有额外成本，因此建议：

1. **前端先 resize**
2. **检测频率限流**
3. **只在桌面启用**
4. **捕获后重检允许使用更高精度输入**

## 9. 资源打包设计

当前 `src-tauri/tauri.conf.json` 已启用：

```json
"bundle": {
  "resources": ["resources/*"]
}
```

因此建议新增：

```text
src-tauri/resources/models/document-boundary.onnx
src-tauri/resources/onnxruntime/windows/...
src-tauri/resources/onnxruntime/linux/...
src-tauri/resources/scanner-yolo-config.json
```

### Windows 资源

- DML-enabled ORT 动态库
- 必要时补充 `DirectML.dll`

### Linux 资源

- ORT 主库
- ORT provider 共享库
- 不强行打包整套 NVIDIA 运行时

目标机仍需自行满足：

- NVIDIA Driver
- CUDA
- cuDNN
- TensorRT（若要走 TensorRT）

## 10. 调试与设置设计

## 10.1 设置项

建议在 `settings-store.ts` 中新增：

- `scannerDetectionBackend: "opencv" | "native-yolo"`
- `scannerNativeYoloLinuxProviderPreference: "tensorrt-cuda-auto" | "cuda-only"`
- 可选 `scannerNativeYoloStrictMode: boolean`

## 10.2 调试项

建议在 `scanner-store.ts` / `ScannerDebugPanel.tsx` 中显示：

- 当前检测后端
- 当前 provider
- 模型是否找到
- session 是否 ready
- 最近一次推理耗时
- 最近一次 fallback reason

## 11. 回退策略

### 推荐默认回退

```text
native-yolo unavailable -> existing OpenCV path
```

### 严格模式

若用户显式开启 strict mode，则：

- native yolo 初始化失败时不自动静默回退；
- UI 直接提示模型/依赖缺失原因。

## 12. 风险清单

### 风险 A：没有可上线模型

这是当前最大风险。  
解决：先冻结运行时设计，模型路线改为“合成数据 + MIDV 微调”。

### 风险 B：Linux 发行复杂

TensorRT/CUDA 依赖目标机环境，部署复杂度显著高于 Windows DirectML。  
解决：保留 CPU fallback，并将 Linux GPU 能力视为“增强能力”而非强依赖。

### 风险 C：IPC 频繁导致卡顿

解决：前端 resize + 限流 + session 缓存。

### 风险 D：Windows DirectML 的长期演进风险

解决：在设计中保留后续切到 WinML 或其他 Windows EP 的可替换空间，但当前不提前切换。

## 6.3 训练方案细化

### 阶段 A：合成数据

自动生成 50k~200k 样本：

- 干净页面内容（PDF 渲染页、LaTeX 页面、公开文本模板）
- 随机背景
- 随机单应变换
- 阴影 / 模糊 / 噪声 / 亮度扰动
- 部分遮挡

自动标签：

- 4 个角点
- 可选一个 document bbox

### 阶段 B：真实数据补强

将 MIDV-500 / MIDV-2019 标注转换为：

```text
class + bbox + 4 keypoints
```

并与 synthetic 数据混训。

### 阶段 C：验证集

使用：

- MIDV 留出集
- SmartDoc

评估：

- 平均角点像素误差
- 角点顺序稳定性
- perspective warp 后输出质量

## 13. 最终建议

### 建议一

**继续做 YOLO，但必须切换到桌面 Rust 原生方案。**

### 建议二

**运行时先行、模型并行准备。**

也就是：

1. 先做 Tauri 原生检测骨架、provider 探测、资源打包位；
2. 再补模型；
3. 最后接入实时预览和拍照后重检。

### 建议三

**模型不要押宝“公共现成通用模型”，也不要继续押宝 OBB。**

应将主计划设为：

```text
YOLO Pose（4 keypoints）
-> synthetic generation
-> MIDV fine-tune
```

## 参考资料

- ORT Rust bindings: https://github.com/open-runtime/ort-rust
- Ultralytics Pose task: https://docs.ultralytics.com/tasks/pose/
- Ultralytics Pose datasets: https://docs.ultralytics.com/datasets/pose/
- ONNX Runtime EP docs: https://onnxruntime.ai/docs/execution-providers/
- DirectML EP: https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- TensorRT EP: https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- CUDA EP: https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
- Tauri docs: https://v2.tauri.app/
