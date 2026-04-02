# 2026-04-02 扫描器原生 YOLO 预览性能优化需求冻结

## 背景

当前桌面端扫描器已经接入原生 YOLO/ORT 配置与设置页绑定，但预览检测链路仍存在明显性能问题：

- 用户反馈桌面端 YOLO 性能过低；
- 现场观测指标为 `ipc wait` 平均达到 `1000ms+`；
- 当前链路位于：
  - `src/components/scanner/ScannerView.tsx`
  - `src/lib/tauri/scanner-detect.ts`
  - `src-tauri/src/scanner_detect.rs`

从仓库当前实现可确认三类结构性损耗：

1. 预览帧会先被编码为 PNG，再通过 Tauri IPC 发送到 Rust，Rust 再重新解码；
2. Rust 原生检测请求虽然接收了 `maxWidth` / `maxHeight`，但当前未真正生效；
3. DocAligner heatmap 后处理在原图尺寸上做 resize + 连通域计算，存在额外 CPU 开销。

## 目标

在不破坏当前桌面端原生 YOLO 配置、provider 选择、现有拍照/后处理结果语义的前提下，极限压榨预览检测性能，优先降低原生 YOLO 实时检测对桌面端 IPC 与主链路延迟的影响。

## 交付物

1. 原生 YOLO 预览检测低延迟路径；
2. Rust 原生检测对处理尺寸限制的真实支持；
3. 更低成本的 heatmap 解码路径；
4. 需求、计划、运行期收据与验证证据。

## 验收标准

1. 预览阶段不再依赖“每帧 PNG 编码 -> IPC -> Rust 解码”的原生 YOLO 调用路径；
2. 原生检测请求真实使用 `maxWidth` / `maxHeight`，而不是忽略；
3. Rust heatmap 后处理明显比原先“升采样到原图后再跑连通域”更轻；
4. 设置页、probe、现有 provider / model 绑定行为不回退；
5. `pnpm exec tsc --noEmit --pretty false` 通过；
6. `cargo check --manifest-path src-tauri/Cargo.toml` 通过；
7. `cargo test --manifest-path src-tauri/Cargo.toml` 通过；
8. 至少补充一条原生 YOLO 性能/冒烟验证证据。

## 约束

- 不引入新的第三方依赖；
- 不重训模型；
- 不修改桌面端配置文件结构；
- 不回退已有 Tauri 设置页真实读写绑定；
- 不为了提速而破坏拍照后的高质量处理链路。

## 非目标

- 本轮不改模型权重；
- 不改 provider 策略；
- 不改高质量 still capture 传输协议；
- 不追求 Web 端 YOLO。

## 推断假设

1. 用户当前优先级是预览实时检测延迟，而不是追求预览阶段最高几何精度；
2. 允许把预览原生检测固定为“低分辨率实时检测，高质量拍照结果保持原逻辑”；
3. 只要最终拍照裁切质量不回退，预览阶段轻微点位抖动可接受。
