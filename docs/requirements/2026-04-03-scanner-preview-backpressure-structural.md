# 2026-04-03 扫描预览背压结构性优化需求

## 背景

用户对上一轮“原生 YOLO 实时链路”优化结果不满意，明确要求继续压榨真实瓶颈，并禁止通过降低分辨率、拉长检测间隔、调整阈值/稳定参数等方式制造假收益。

当前交接材料已经指出：用户最初看到的 `ipc wait` 1000ms+ 指标来自 preview streaming path，而不是 `scanner_detect.rs` 自身的推理耗时。因此本轮目标必须转向 preview packet 在 renderer 侧的 decode / render / queue backlog 问题。

## 冻结目标

在**不改参数**的前提下，收敛并消除 renderer 侧“按到达顺序处理全部旧 preview packet”导致的主线程背压，把预览链路恢复为真正的 latest-only 语义。

## 交付物

1. 一项或多项针对 preview decode/render 热路径的结构性优化；
2. 说明旧实现为何会在主线程繁忙时累积陈旧 packet 的证据；
3. 对应验证输出与运行时回执。

## 硬约束

- 不降低检测分辨率；
- 不拉长检测间隔；
- 不调整阈值、稳定参数、provider 顺序来伪造收益；
- 不改变用户可见的扫描功能与现有回退链路；
- 不对真实设备收益做无证据声明。

## 关键判断

### 已确认问题面

- `src/lib/tauri/adb.ts` 已把 frame handoff 从 `requestAnimationFrame` 改成 `queueMicrotask()`；
- 但 `src/lib/scanner/frame-source.ts` 仍会对每个已进入 renderer 事件队列的 packet 执行同步 decode；
- 一旦 decode/render 主线程成本超过预算，队列里的旧 packet 仍会被顺序消费，造成 `lastIpcMs` 持续抬高；
- 这属于**背压调度错误**，不是参数问题，也不是 native YOLO model compute 本身的问题。

### 本轮目标根因

把 preview packet 消费从“每个到达任务都解码”改成“下一次 decode 前只保留最新 packet”，让主线程在繁忙时主动丢弃陈旧预览帧，而不是把延迟变成排队时间。

## 验收标准

1. `frame-source` 存在明确的 latest-only decode drain gate，而不是每个 packet 直接同步解码；
2. preview canvas 路径补上足够的本地观测，以便区分 decode 成本与 draw 成本；
3. `pnpm exec tsc --noEmit --pretty false` 通过；
4. 生成一份背压基准，对比 eager 顺序消费与 latest-only 调度在 IPC backlog 上的差异；
5. 输出 requirement / plan / execution / cleanup artifacts。

## 非目标

- 不在本轮重写 OpenH264 / Tauri channel / Android camera server；
- 不把 synthetic benchmark 冒充 real-device benchmark；
- 不重新进入“把参数调小一点”的路线。
