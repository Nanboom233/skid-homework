# 2026-04-03 扫描器原生 YOLO 实时链路结构性优化需求冻结

## 背景

上一轮已经完成：

- 预览原生检测从 PNG 往返切到 RGBA 请求；
- Rust 原生检测真实支持 `maxWidth` / `maxHeight`；
- heatmap 后处理改为 heatmap 空间解码。

用户新增强约束：

> 不允许通过修改参数做“假优化”。

因此本轮优化必须明确排除：

- 调低检测分辨率
- 拉长检测间隔
- 调整阈值/稳定帧数/滤波参数
- 通过更激进的 fallback 策略掩盖真实性能问题

## 目标

继续压榨桌面端原生 YOLO 实时链路，但仅允许做 **结构性优化**：

1. 消除每帧重复的配置/资源/模型解析成本；
2. 消除重复图像变换与无谓拷贝；
3. 用基准验证真实收益，而不是靠参数变化制造表面提速。

## 验收标准

1. 本轮不得通过修改检测频率、处理尺寸、阈值等参数拿性能；
2. 至少实现一项明确的结构性优化并进入主路径；
3. 需要补充 benchmark 证据，证明收益来自结构变化；
4. `pnpm exec tsc --noEmit --pretty false` 通过；
5. `cargo check --manifest-path src-tauri/Cargo.toml` 通过；
6. `cargo test --manifest-path src-tauri/Cargo.toml` 通过。

## 本轮允许的方案类型

- runtime/config/session metadata cache
- 去除每帧 probe/config 读取
- 去除重复 resize / decode / materialize
- 减少不必要的中间对象分配
- 让 benchmark 能区分“结构优化收益”和“参数变更收益”

## 本轮明确禁止

- 继续把 `320x180`、`120ms`、阈值等再往下压
- 通过改 provider 顺序掩盖真实热路径
- 只在 benchmark 命令里改参数，不改真实主链路
