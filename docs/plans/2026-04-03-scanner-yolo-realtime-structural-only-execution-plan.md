# Scanner YOLO Realtime Structural-Only Execution Plan

## Goal

在不触碰检测参数的前提下，继续降低桌面端原生 YOLO 实时链路成本。

## Internal Grade

`L`

## Frozen Constraint

不允许以下“伪优化”进入本轮实现：

- 更低检测分辨率
- 更长轮询/检测间隔
- 更松阈值
- 更激进 fallback

## Repo-grounded Hotspots

- `src-tauri/src/scanner_detect.rs`
  - detect 路径仍会重复走部分 probe/config/model 解析
- `src-tauri/src/bin/scanner-native-yolo-benchmark.rs`
  - 当前 benchmark 只比较 transport/path，不足以区分后续结构性收益

## Waves

### Wave 1: Runtime detection context cache

目标：

- 每帧 detect 不再重复做资源根路径选择、配置文件读取、资源存在性推导与模型选择。

### Wave 2: Remove redundant per-request work

目标：

- 尽量减少 detect 路径上的重复 materialize / clone / state assembly。

### Wave 3: Benchmark proof

目标：

- benchmark 输出要能证明收益来自结构性变化，而不是参数变化。

## Verification

```powershell
pnpm exec tsc --noEmit --pretty false
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-benchmark -- .github/images/skidhw-thumbnail-new.png --iterations 3
git diff --check
```

## Rollback Rules

- 若 cache 引入错误的 stale config/session 行为，优先保持正确性并回退该 cache；
- 不修改用户已有 `.gitignore`；
- 不接触未被证实为真实 hotspot 的参数位。

## Cleanup Expectations

- 写入新 session 的 requirement/plan/execution/cleanup receipts；
- 总结中明确区分“结构优化收益”和“上一轮参数位不变”。
