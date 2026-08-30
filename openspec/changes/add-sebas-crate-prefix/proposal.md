# 统一 workspace 成员 crate 命名为 sebas- 前缀

## Why

workspace 成员 crate 命名过于通用（`router`、`gateway`、`feishu`、`webui`、`acp-claude`），缺乏项目归属感，在依赖图、构建日志与文档中难以一眼辨识归属，也容易与生态中同名概念混淆。统一加 `sebas-` 前缀使命名空间清晰一致。

## What Changes

- 重命名 5 个成员 crate 的包名与目录名（目录与包名保持一致）：
  - `router` → `sebas-router`
  - `feishu` → `sebas-feishu`
  - `acp-claude` → `sebas-acp-claude`
  - `gateway` → `sebas-gateway`
  - `webui` → `sebas-webui`
- 同步更新：
  - 根 `Cargo.toml` 的 workspace members 与依赖声明
  - 各 crate 间路径依赖声明与 `use` 导入（连字符转下划线，如 `use sebas_router::`）
  - `Dockerfile` 中对 crate 目录的 COPY 路径
- 文档同步：README、docs/ 中出现的 crate 名引用；活跃 OpenSpec 变更（`add-core-session-channel` 等）中的 crate 名引用。

## Capabilities

无新增/修改能力 —— 本变更为纯内部重构：二进制名、CLI 子命令、配置格式、对外 API 均不变，无 spec 级行为变化。已在 `.openspec.yaml` 设置 `skip_specs: true`。

## Non-goals

- 不改 CLI 子命令名（`sebas gateway` 等保持原样）
- 不改 `config.toml` 配置段名（`[gateway]`、`[feishu]`、`[watchdog.webui]` 等）
- 不改 `state/` 下运行时文件名（`gateway-usage.jsonl` 等）
- 不改 `xtask`（构建工具，保持惯例命名）与测试用 `fake-claude` bin
- 不改根包 `sebas`（已含前缀）与根 bin `sebas`
- 除导入路径、清单与目录名外，不改任何运行时代码逻辑

## Impact

- **代码**：5 个 crate 的 `Cargo.toml`；约 286 处 `use` 导入；根 `Cargo.toml`
- **构建**：`Dockerfile` COPY 路径、workspace members、路径依赖
- **文档**：README、docs/、活跃变更 `add-core-session-channel` 的设计/任务/spec 引用
- **兼容性**：crate 均未发布到 crates.io，无外部消费者；对最终用户零影响
