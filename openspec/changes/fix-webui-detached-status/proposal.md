## Why

release 构建实测（沙箱双形态）：核心链路两种部署形态都通，但 detached（watchdog 正式拓扑）下 webui 控制面存在三处"上报不实"，操作员看到的控制台与真实状态不符——provider 恒显示"未配置"、缺凭据的拒绝被误报成"核心不可达"、未知执行体静默落 ACP。这正是"release 启动后功能不太正常"体感的直接来源之一。

## What Changes

- detached 形态 `/api/settings`（及 `/api/gateway`、`/api/about`）不再使用 `GatewayInfo::default()` 占位：provider 列表从状态库读取（webui 进程已有状态库访问，add-state-store 既有通道），`listen`/`debug`/`has_auth` 从配置解析；composer 的 provider 标签如实反映已配置的 provider
- `SessionRejection::Unavailable` 不再是"核心不可达"一揽子文案：区分"通道不可达"与"执行体不可用（如 native 缺凭据）"，拒绝原因如实呈现给操作员
- Spawn 的 `backend` 提示校验：显式给出但不在已知集合（native/acp 及 acp:<kind> 前缀）的值返回 typed rejection，不再静默按 ACP 创建会话（缺省不传仍默认 acp，向后兼容不变）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：detached 形态下 settings/gateway/about 路由的 provider 数据来源与如实呈现；会话拒绝文案按真实原因区分（通道不可达 vs 执行体不可用）
- `core-session-channel`：Spawn 请求 `backend` 提示的校验语义——未知值必须 typed rejection 且不创建会话

## Impact

- 代码：`src/webui_cmd.rs`（GatewayInfo 装配）、`sebas-webui/src/api.rs` + `sebas-webui/src/state.rs`（settings 数据源）、`sebas-webui/src/session_backend.rs`（SessionRejection 变体/Display）、`src/agent_backend.rs` 与 `src/core_channel/server.rs`（unknown backend 校验）
- 协议：无新消息类型；仅校验语义收紧（显式非法值从"静默回退"变为"拒绝"）
- 兼容：不传 `backend` 的旧客户端行为不变；auth 开关、绑定门控均不动

## Non-goals

- detached 下 `execution_bodies`/模型面（`available_models`/SetModel）/审批往返——归在途变更 `wire-webui-sebas-agent-e2e`（tasks 1.x–4.x）
- gateway 运行时动态信息（随机端口等）经通道上报——保持配置静态解析即可
- 原生内核凭据注入机制的变更
