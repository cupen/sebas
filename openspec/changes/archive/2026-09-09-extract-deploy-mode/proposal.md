## Why

`feishu-option` 把三件不同域的事缝在一起：`[feishu] enabled` 配置开关（纯飞书配置语义）、webui 主控部署形态（watchdog 的默认服务启停策略，牵涉 webui/core/im）、双通道共享会话状态（中立的 ChannelKey 会话权威汇聚）。glossary 命名规则把 `-option` 定为「配置开关」补缀——部署形态与共享状态不是开关语义，挂在 feishu 前缀下也让「webui 主控」这个与飞书无关的部署决策误导读者。

## What Changes

- **新建 `deploy-mode` capability**：承接收「webui 主控部署形态」（watchdog 默认 webui 启、core 停、im 跟随 feishu 可显式覆盖）与「双通道共享会话状态」（`web`/`feishu` 经 ChannelKey 汇聚到单一会话权威、前缀不由核心特判）。
- **`feishu-option` 收窄**：REMOVED 两条部署形态 requirement（webui 主控部署形态、双通道共享会话状态），保留「Feishu 显式启用开关」为唯一 requirement，Purpose 同步收窄为飞书接入配置开关。
- `feishu-option` 目录名保留（批次 E 的 feishu-* 族收窄计划已覆盖）。

## Capabilities

### New Capabilities
- `deploy-mode`: 部署形态与多通道共享状态（watchdog 服务启停策略、ChannelKey 会话汇聚）。

### Modified Capabilities
- `feishu-option`: 移除部署形态语义，收窄为飞书接入配置开关

## Impact

- glossary「主控」词条引用 `(feishu-option)` 改指 `deploy-mode`；watchdog/webui 若引用 feishu-option 部署语义一并核对。
- im-service/channels 与双通道共享状态的关系保持。
- **Non-goals**：不改任何源码行为；不改 `[feishu] enabled` 半配置/env 矩阵语义；不重排 watchdog 既有 requirement。
