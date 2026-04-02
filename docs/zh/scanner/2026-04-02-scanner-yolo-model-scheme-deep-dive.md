# 2026-04-02 扫描器 YOLO 模型方案深挖

## 一句话结论

如果目标是给扫描器输出 **4 个有序页面角点**，那么当前最现实、最低成本、最适合落到 Rust + ONNX Runtime 的路线是：

```text
YOLO Pose（4 keypoints）
-> synthetic page compositing
-> MIDV-500 / MIDV-2019 mixed fine-tune
-> ONNX export
-> Tauri Rust inference
```

## 为什么不是 OBB

OBB 的问题不是“没有 4 点”，而是这 4 点描述的是 **旋转矩形**。

但扫描器中的页面在相机透视下往往是：

- 梯形
- 不等边四边形
- 一般 projective quadrilateral

所以：

- OBB 的点可能只是“最佳旋转包围框”的顶点；
- 不一定是真实页面四角；
- 透视越强，失配越明显。

结论：

> OBB 可做实验项，但不适合做主模型方案。

## 为什么 YOLO Pose 最合适

### 1. 与业务契约直接一致

扫描器下游就是要：

```ts
[topLeft, topRight, bottomRight, bottomLeft]
```

YOLO Pose 可以直接把 4 个 keypoints 定义成这 4 个语义点。

### 2. 与公开数据更匹配

MIDV-500 / MIDV-2019 天然更接近“四角点标注”，转换成 pose 标签比做 segmentation mask 更便宜。

### 3. 工程成本最低

与 segmentation 相比：

- 不需要额外 mask 标注
- 不需要 contour -> quad 的重后处理
- 可以直接从关键点输出进入现有透视裁切

### 4. 导出链路清晰

Ultralytics 官方支持：

- Pose task
- 自定义 keypoint 数量
- ONNX export

## 为什么 segmentation 仍然值得保留

Segmentation 不是被否决，而是：

- **几何上更强**
- **工程上更重**

它最适合作为：

- pose 精度不够时的升级路线
- 复杂阴影/遮挡/卷边场景下的备选路线

推荐角色：

> 高保真备选，而不是第一版主线。

## 推荐标签设计

## 1. keypoint 语义固定

```text
0 = top-left
1 = top-right
2 = bottom-right
3 = bottom-left
```

这样能避免运行时再做复杂排序。

## 2. Pose 配置

推荐按 Ultralytics pose 数据格式组织：

```yaml
kpt_shape: [4, 3]
```

其中每个点：

- `x`
- `y`
- `visible`

## 3. 训练输入

第一版建议：

- `640 x 640`
- 小模型族：`yolo*n-pose`

因为桌面推理虽然比浏览器宽松，但仍要考虑：

- Windows DirectML 首帧速度
- Linux TensorRT/CUDA 批量优化前的初始延迟
- 实时预览的调用频率

## 推荐训练路线

## 阶段 A：合成数据（主量）

自动生成 50k~200k 样本：

- 页面内容：PDF 渲染页 / 文本模板 / 公开文档素材
- 背景：桌面、木纹、布料、手持场景
- 变换：随机 homography
- 扰动：
  - blur
  - noise
  - brightness
  - shadow
  - partial occlusion
  - perspective extremes

优点：

- 零人工标注成本
- 标签天然准确
- 能快速覆盖极端姿态

## 阶段 B：MIDV-500 / MIDV-2019（真实补强）

做一个转换脚本，把公开标注转成：

```text
class + bbox + 4 keypoints
```

作用：

- 把模型从“合成世界”拉回真实手机/相机域
- 增强光照、反光、边缘模糊的泛化能力

## 阶段 C：SmartDoc（验证/补充）

不建议作为主训练集，但适合作为：

- 老旧移动端场景补充
- 泛化验证集

## 成本判断

### 为什么这条路便宜

和手工标四角点相比：

- synthetic 路线几乎没有标注成本
- MIDV / SmartDoc 是公开数据
- 主要成本变成：
  - 转换脚本
  - 数据生成脚本
  - 训练与验证时间

所以这条路的成本中心在 **工程时间**，而不是 **持续的标注采购成本**。

### 粗略工程量

第一版可用模型的合理投入大致是：

- 数据生成器：1~2 周
- MIDV 转换脚本：2~3 天
- 训练/验证：3~5 天
- 导出/联调：约 1 周

总计：

> 约 4~6 人周可以拿到第一版可部署模型

## 对运行时的影响

既然模型方案改成 YOLO Pose，那么运行时也应同步收敛：

### Rust 侧

- `scanner_detect.rs`
- 预处理按 pose 模型输入尺寸
- 输出直接解码 4 个 keypoints

### 前端侧

- 调试信息显示：
  - model task = pose
  - keypoint confidence
  - provider
  - inference ms

### 平台侧

- Windows：DirectML -> CPU
- Linux：TensorRT -> CUDA -> CPU

## 最终推荐

### 第一选择

**YOLO Pose（4 keypoints）**

### 第二选择

**Segmentation + quad fitting**

### 明确不推荐作为主路线

- OBB
- 普通 detect

## 对公开模型的补充修正

之前“没有公共模型”这个表述过于粗。

更准确的说法是：

- 公开的 YOLO document-scanner / document-detection 示例**存在**
- 但它们大多输出的是：
  - 单个 document bbox
  - 或 bbox + OpenCV refinement

所以它们的问题不是“完全不存在”，而是：

- **输出契约不够强**
- **很少直接给 4 个有序角点**
- **难以直接替代我们要的四角点 stage-1 方案**

因此最合理的定位是：

- 把公开 bbox 模型当作 baseline / 过渡方案
- 把 `YOLO Pose 4-corner` 作为主路线

再补一条更精确的工程判断：

- 当前仓库已经有一个可公开落盘、可真实推理的四角点 baseline：
  - `docaligner-fastvit-sa24`
- 但它的角色是：
  - **当前可交付的 stage-1 过渡模型**
  - 不是“最终 YOLO 主模型已经到位”的替代说法

## 参考

- Ultralytics Pose Task: https://docs.ultralytics.com/tasks/pose/
- Ultralytics Pose Dataset Guide: https://docs.ultralytics.com/datasets/pose/
- Ultralytics Export: https://docs.ultralytics.com/modes/export/
- MIDV-500 official whitepaper: https://smartengines.com/wp-content/uploads/2020/04/datasets-of-id-documents-midv-500.pdf
- MIDV-500 arXiv: https://arxiv.org/abs/1807.05786
- MIDV-2019 extension mention: same Smart Engines whitepaper above
