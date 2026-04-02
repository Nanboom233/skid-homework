# 2026-04-02 Scanner YOLO 设置页真实绑定总结

## 结论

本轮把桌面端原生 YOLO/ORT 配置从“内置只读资源”推进到了“前端可直接编辑的真实配置文件绑定”。

当前行为是：

- 读取顺序：
  - 先读 `app_config_dir()/scanner-yolo-config.json`
  - 若不存在，再回退到 `src-tauri/resources/scanner-yolo-config.json`
- 写入位置：
  - 统一写入可写的 `app_config_dir()/scanner-yolo-config.json`
- 设置页：
  - 桌面端新增原生运行时配置卡
  - 可直接编辑 stage/task、主模型、公共基线模型、Windows Provider、Linux Provider 列表与备注
  - 保存后会立即重新 probe，刷新当前 provider / model / runtime 状态

## 本轮改动

### 1. Rust 侧真实配置读写

文件：

- `src-tauri/src/scanner_detect.rs`
- `src-tauri/src/lib.rs`

新增能力：

- `tauri_scanner_read_yolo_config`
- `tauri_scanner_write_yolo_config`
- 配置 override 解析
- 保存后清理 ORT session cache
- probe / detect 统一走配置驱动的模型与 provider 元数据

同时修正了：

- `preferred_provider` 计算顺序错误
- provider 顺序仍停留在旧静态逻辑的问题
- 底部测试仍引用旧 `ScannerModelVariant` 静态 spec 的问题

### 2. 前端桥接

文件：

- `src/lib/tauri/scanner-detect.ts`

新增内容：

- `TauriScannerYoloConfig*` 类型
- `readTauriScannerYoloConfig()`
- `writeTauriScannerYoloConfig()`
- probe 返回中的 `configSource` / `configPath`

### 3. 设置页绑定

文件：

- `src/components/settings/SettingsPage.tsx`

新增内容：

- 桌面专用“原生桌面运行时配置”卡片
- 真实加载/保存按钮
- 当前配置来源、当前解析路径、可写覆盖路径展示
- probe 状态展示：
  - runtime ready
  - session ready
  - preferred provider
  - selected model
  - probe message
- 保存/加载失败 toast

### 4. 文案与验证脚本

文件：

- `public/locales/en/commons.json`
- `public/locales/zh/commons.json`
- `scripts/test-scanner-yolo-settings-binding.ps1`

## 验证命令

```powershell
pnpm exec tsc --noEmit --pretty false
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo run --manifest-path src-tauri/Cargo.toml --bin scanner-native-yolo-probe
pwsh scripts/test-scanner-yolo-settings-binding.ps1
```

## 构建环境结论

- 当前仓库的前端构建链最终可通过：
  - `pnpm build`
- 但在本机这次验证环境里，`pnpm build` 需要在提权 PowerShell 中执行。
- 我额外验证过：
  - 不提权时，Next build 在当前环境会出现 `spawn EPERM`
  - 试图通过 `experimental.workerThreads` 绕过会在 Next 静态导出阶段触发 `DataCloneError`
  - 因此最终保留的是：
    - `package.json` 里先执行 `tsc --noEmit`
    - `next.config.js` 里跳过 Next 自带的重复类型检查
    - 不保留 `workerThreads` workaround

## 官方依据

本轮额外用 grok-search 复核了 ORT 平台约束，设计与当前仓库目标一致：

- Rust `ort` crate execution provider feature flags：
  - https://docs.rs/ort/latest/ort/
  - https://github.com/pykeio/ort
- DirectML 官方要求与 session 限制：
  - https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html
- ORT execution provider shared library 约定：
  - https://onnxruntime.ai/docs/build/eps.html
- CUDA / TensorRT 官方文档：
  - https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html
  - https://onnxruntime.ai/docs/execution-providers/TensorRT-ExecutionProvider.html

其中与当前实现最直接相关的结论是：

- Windows DirectML 仍要求顺序执行，并禁用 memory pattern
- Linux 的 TensorRT / CUDA provider 仍按 shared library 方式分发
- 当前仓库在 Windows 走 DirectML、Linux 走 TensorRT/CUDA 的方向没有偏
