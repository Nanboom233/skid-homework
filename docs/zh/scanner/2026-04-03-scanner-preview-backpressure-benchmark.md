# 2026-04-03 扫描预览背压调度基准

- 状态: `passed`
- 生成时间: `2026-04-02T20:03:35.446Z`
- 场景: `synthetic-backpressure`
- 输入: `640x360 @ 30fps`
- 模拟下游成本: `36ms / frame`

## 结论

- eager 平均 IPC: `1123.774` ms
- latest-only 平均 IPC: `67.169` ms
- eager P95 IPC: `2078` ms
- latest-only P95 IPC: `95` ms
- latest-only 丢弃的陈旧 packet: `85`

## 模式对比

| Mode | Produced | Processed | Dropped | Effective FPS | IPC Avg | IPC P95 | Decode Avg |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| eager | 239 | 239 | 0 | 23.72 | 1123.774 | 2078 | 4.896 |
| latest-only | 239 | 154 | 85 | 19.25 | 67.169 | 95 | 7.078 |
