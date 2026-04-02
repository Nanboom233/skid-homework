# 2026-04-03 扫描预览背压结构性优化总结

## 本轮目标

这轮不是继续调参数，而是正面处理 preview streaming path 在 renderer 侧的**背压失控**问题：

- 用户最早看到的 `ipc wait` 1000ms+ 指标来自 preview packet backlog；
- 不是 native YOLO model compute 本身；
- 也不是再把分辨率、检测间隔、阈值调小就能合理解释的问题。

## 根因结论

`src/lib/tauri/adb.ts` 虽然已经把最新 packet handoff 改成了 `queueMicrotask()`，但这只能保证**单个 task 内**拿最新 packet，挡不住**已经排进 renderer 事件队列**的 channel callback。

旧链路的问题是：

1. Tauri channel callback 仍会一个个进入 renderer 任务队列；
2. `frame-source.ts` 对每个已排队 packet 都同步做 `decodeFramePacketToRgba()`；
3. 一旦 decode/render 主线程成本超过预算，旧 packet 仍会被顺序消费；
4. 最终体现为 `lastIpcMs` / `ipc wait` 持续升高，因为它实际上开始包含“排队等主线程”的时间。

换句话说，真正缺的不是“有没有 latest-only handoff”，而是**latest-only 的 decode drain gate**。

## 实际代码改动

### 1. `src/lib/scanner/frame-source.ts`

新增 preview latest-only drain gate：

- channel callback 不再直接同步 decode；
- 只更新 `queuedStreamPacket`；
- decode 延迟到下一次 macrotask；
- 如果等待 decode 期间又来了新 packet，只保留最新的一份；
- stop/reset/cleanup 时会清空 queued packet 和 drain timer。

这意味着：

- renderer 忙的时候，旧 preview frame 会被主动丢弃；
- 不再把“历史帧债务”积成上百毫秒甚至秒级 IPC wait；
- 语义上和 preview 本来就需要的 latest-only 显示一致。

### 2. `src/components/scanner/ScannerView.tsx`

对 preview draw hot path 做了两项轻量化收口：

- 复用 visible canvas / frame-buffer canvas 的 2D context；
- 给 context 增加 `alpha: false`、`desynchronized: true` hint；
- portrait draw 路径去掉无必要的 `clearRect()`；
- 低频记录 `canvasDrawMs`，后续可以把 decode / draw 成本分开看。

## 验证

已实际运行：

```powershell
pwsh -NoProfile -File .\scripts\test-scanner-preview-backpressure-structural.ps1
```

结果：

- `pnpm exec tsc --noEmit --pretty false` 通过；
- `pnpm build` 通过；
- synthetic backpressure benchmark 通过；
- `git diff --check` 只有既有 LF/CRLF warning，没有新增 whitespace 错误。

新增测试脚本：

- `scripts/test-scanner-preview-backpressure-structural.ps1`

## Synthetic benchmark 结果

输入场景：

- `640x360 @ 30fps`
- 模拟 renderer 每帧额外下游成本：`36ms`
- producer 在独立 worker thread 持续送 packet，模拟 Tauri channel callback 持续到达

结果：

| Mode | Produced | Processed | Dropped | Effective FPS | IPC Avg | IPC P95 | Decode Avg |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| eager | 239 | 239 | 0 | 23.72 | 1123.774 ms | 2078 ms | 4.896 ms |
| latest-only | 239 | 154 | 85 | 19.25 | 67.169 ms | 95 ms | 7.078 ms |

关键含义：

- **decode 本身不是 backlog 的主因**，即使 decode 在更重负载下升到 `4.896ms ~ 7.078ms`，真正把 `ipc wait` 炸高的仍然是排队债务；
- 真正把 `ipc wait` 炸高的是“顺序偿还所有旧 packet 债务”；
- 引入 latest-only drain gate 后，平均 IPC backlog 从 `1123.774ms` 压到 `67.169ms`；
- P95 从 `2078ms` 压到 `95ms`；
- 代价是主动丢掉 `85` 个已经过时的 preview packet —— 这正是 preview 场景应该接受的策略。

## 边界说明

这份 benchmark 是 **synthetic backpressure benchmark**，不是 real-device benchmark。

它证明的是：

- 当前调度策略在“producer 持续送帧、renderer 主线程有额外负载”时，会不会把 IPC 延迟炸成 backlog；
- latest-only drain gate 是否能从调度层面把旧帧债务切断。

它**不能**替代下一步 real-device 复测。

## 下一步最值得做的事

1. 在真实设备上复测 `pollWaitMs / jsDecodeMs / canvasDrawMs`；
2. 如果 `canvasDrawMs` 仍显著偏高，再继续考虑 preview render 的 worker / OffscreenCanvas 路线；
3. 如果 native-yolo 仍在主线程路径里拉高负载，再考虑把 detect 输入从 RGBA 进一步下沉到 preview packet / I420 级别。
