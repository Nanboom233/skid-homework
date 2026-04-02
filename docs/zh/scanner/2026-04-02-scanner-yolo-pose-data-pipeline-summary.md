# 2026-04-02 Scanner YOLO Pose 数据管线总结

## 结论

在当前没有可直接落地的公共 `4-corner YOLO pose` 成品模型前，最稳妥的主路线是：

```text
synthetic page compositing
-> MIDV-500 / MIDV-2019 quad conversion
-> Ultralytics YOLO Pose (4 keypoints)
-> ONNX export
-> Windows DirectML / Linux TensorRT-CUDA
```

本轮已经把这条路线收敛成可执行的数据准备骨架，而不是只停留在文档层面。

## 本轮新增产物

### 1. MIDV -> YOLO Pose 转换脚本

文件：

- `scripts/scanner_yolo/convert_midv_to_yolo_pose.py`

作用：

- 读取 MIDV 风格的 `quad` 标注 JSON
- 自动寻找同名图像
- 统一角点顺序为：
  - `top-left`
  - `top-right`
  - `bottom-right`
  - `bottom-left`
- 生成 Ultralytics pose 标签：

```text
class cx cy w h x1 y1 v1 x2 y2 v2 x3 y3 v3 x4 y4 v4
```

### 2. Synthetic Pose 数据生成脚本

文件：

- `scripts/scanner_yolo/generate_synthetic_pose_dataset.py`

作用：

- 从页面素材与背景素材自动合成训练样本
- 自动生成四角点标签
- 支持：
  - 随机透视
  - 阴影
  - 轻度模糊
  - 遮挡
  - 亮度/对比度扰动
- 直接输出 Ultralytics pose 所需的 `images/<split>` 与 `labels/<split>`

### 3. 公共工具模块

文件：

- `scripts/scanner_yolo/common.py`

作用：

- 统一 quad 排序
- 统一 bbox / keypoint 归一化
- 统一标签写出
- 统一 `dataset.yaml` 生成

### 4. 测试脚本

文件：

- `scripts/test-scanner-yolo-data-pipeline.ps1`

作用：

- 自动生成 smoke fixtures
- 编译 Python 脚本
- 跑 MIDV 转换
- 跑 synthetic 生成
- 覆盖：
  - 有背景素材分支
  - 无背景素材时的 procedural background 分支
- 校验输出数量与标签字段数

## 输出契约

生成的数据集统一写成：

```text
images/train
labels/train
dataset.yaml
```

其中 `dataset.yaml` 为：

```yaml
path: .
train: images/train
val: images/val
test: images/test
kpt_shape: [4, 3]
flip_idx: [1, 0, 3, 2]
names:
  0: document
```

关键点顺序固定为：

```text
0 = top-left
1 = top-right
2 = bottom-right
3 = bottom-left
```

这样能直接对齐当前 Scanner 的 `Point[] | null` 契约，避免运行时再做复杂排序猜测。

## 推荐训练命令

当数据准备完成后，推荐先从小模型开始：

```powershell
yolo pose train model=yolo11n-pose.yaml data=<dataset.yaml> epochs=100 imgsz=640 batch=16
```

说明：

- 若要加载预训练 pose 权重再改头部，可以改成：

```powershell
yolo pose train model=yolo11n-pose.pt data=<dataset.yaml> epochs=100 imgsz=640 batch=16
```

- 但对 `kpt_shape: [4, 3]` 的任务，更稳妥的是基于 pose YAML 重新建模，再迁移权重或重新训练。

## 推荐导出命令

### ONNX

```powershell
yolo export model=<best.pt> format=onnx imgsz=640 simplify=True opset=17 dynamic=False
```

### TensorRT

```powershell
yolo export model=<best.pt> format=engine imgsz=640 half=True workspace=4 dynamic=False
```

## 部署映射

导出后的部署合同仍然保持：

- Windows：
  - `ONNX -> ORT DirectML`
- Linux：
  - `ONNX / TensorRT -> ORT TensorRT -> CUDA -> CPU`

## 官方参考

- Ultralytics pose dataset format:
  - https://docs.ultralytics.com/datasets/pose/
- Ultralytics export:
  - https://docs.ultralytics.com/modes/export/
- ONNX Runtime DirectML EP:
  - https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- ONNX Runtime TensorRT EP:
  - https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html
- ONNX Runtime CUDA EP:
  - https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
- MIDV-500 paper:
  - https://arxiv.org/abs/1807.05786

## 当前边界

本轮没有直接生成最终 YOLO 模型文件，也没有在仓库内引入 `ultralytics` 训练依赖。原因很明确：

- 当前仓库重点仍是桌面端产品代码，不适合把重训练依赖直接混入应用 runtime
- 先把数据准备和训练契约固定，再把训练放到独立环境中执行，边界更清楚

所以本轮的正确定位是：

> 已把“YOLO Pose 4-corner”从概念方案推进到可执行的数据准备骨架；  
> 下一步是喂真实 MIDV 数据和 synthetic 素材，跑训练与导出。

## 本轮验证

已验证脚本：

```powershell
python -m compileall scripts/scanner_yolo
pwsh scripts/test-scanner-yolo-data-pipeline.ps1
```

验证点：

- MIDV 转换成功输出标签
- synthetic 生成成功输出图像与标签
- procedural background 分支成功输出图像与标签
- 标签字段数符合 `YOLO Pose 4 keypoints` 预期
- `dataset.yaml` 成功落盘
