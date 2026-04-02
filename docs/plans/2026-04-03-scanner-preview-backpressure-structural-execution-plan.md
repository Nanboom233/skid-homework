# 2026-04-03 扫描预览背压结构性优化执行计划

## Internal Grade

- `L`

## 执行策略

### Wave 1 — 根因与证据冻结

1. 复核交接材料、`frame-source.ts`、`adb.ts`、`ScannerView.tsx`；
2. 明确旧链路的问题不是“有没有 latest-only handoff”，而是“latest-only 只发生在 task 内，无法消化已经排队的 channel callback”；
3. 准备 synthetic backpressure benchmark，专门比较：
   - eager 顺序消费
   - latest-only drain gate

### Wave 2 — 代码改动

1. 在 `src/lib/scanner/frame-source.ts` 加入 latest-only preview packet drain gate：
   - channel callback 只更新最新 packet；
   - 真正 decode 延后到下一次 macrotask；
   - decode 完成后如果期间又收到更新，只继续处理最新 packet；
2. 在 `src/components/scanner/ScannerView.tsx` 轻量化 preview draw hot path：
   - 复用 canvas context；
   - 去掉 portrait draw 里无必要的 `clearRect`；
   - 以低频率记录 `canvasDrawMs` 供后续对账；
3. 不触碰检测参数与 native YOLO runtime 选择逻辑。

### Wave 3 — 验证与收尾

1. 运行 `pnpm exec tsc --noEmit --pretty false`；
2. 运行 synthetic backpressure benchmark，产出 JSON + Markdown；
3. 写 execution / cleanup receipts；
4. 输出中文总结，明确：
   - 真实改动点；
   - synthetic benchmark 的边界；
   - 尚未覆盖的 real-device follow-up。

## 验证命令

```powershell
pnpm exec tsc --noEmit --pretty false
node .\.tmp\scanner-preview-backpressure-benchmark.mjs --duration-ms 8000 --fps 30 --downstream-ms 36 --json outputs/runtime/vibe-sessions/20260403-scanner-preview-backpressure-001/benchmark-preview-backpressure.json --md docs/zh/scanner/2026-04-03-scanner-preview-backpressure-structural-summary.md
```

## 回滚策略

- 若 preview 流生命周期异常，则回滚 `frame-source.ts` 的 latest-only drain gate；
- 若 preview 画布出现渲染异常，则回滚 `ScannerView.tsx` 的 canvas context 复用与 metric 逻辑；
- 不回滚用户已确认有效的 native YOLO cache / transport handoff 改动。

## Cleanup Expectations

- 保留 requirement / plan / benchmark / receipt artifacts；
- 清理本轮新增的临时 benchmark stdout/stderr；
- 不引入新的常驻 node 子进程；
- 最终交付前复核 `git diff --check` 与工作区变更列表。
