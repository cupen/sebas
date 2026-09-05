## Context

实测（release 沙箱双形态）确认三处上报不实，根因各自独立：

1. `src/webui_cmd.rs` 启动 standalone webui 时向 `sebas_webui::run_*` 传入
   `GatewayInfo::default()`——provider 列表、listen、debug、has_auth 全部为
   空占位；in-process 路径（`src/run.rs::build_gateway_info`）则从
   `GatewayConfig` 正常装配。
2. `SessionRejection::Unavailable` 是"请求无法送达会话权威"的兜底变体，
   Display 为"核心不可达: {cause}"；`DualSessionBackend` 对 native 缺凭据的
   拒绝复用了该变体，导致文案误报。
3. `DualSessionBackend` 路由 Spawn 提示时，未知 `backend` 值走"缺省=acp"分支
   静默建成 ACP 会话。

约束：`wire-webui-sebas-agent-e2e`（在途）负责 detached 的
execution_bodies/模型/审批面；本变更不碰那三块，避免同文件大范围冲突。
状态库引擎与通道状态方法（add-state-store 5.1/5.2）已落地，
`backend.state_snapshot(key)` 缝隙在两种形态都可用。

## Goals / Non-Goals

Goals：三种不实上报各自变真；同一配置两种部署形态 UI 一致；旧客户端
（不带 backend 字段）行为零变化。

Non-Goals：不动通道消息集（无新消息类型）；不动 auth/绑定门控；不给
native 补凭据注入机制。

## Decisions

### D1：detached 的 GatewayInfo 装配 = 配置解析 + 状态库 provider 真源

- 静态事实（`listen`、`debug`、`has_auth`）：webui 已持有同一份 config 原文，
  直接 `GatewayConfig::parse` 后映射，与 `build_gateway_info` 同逻辑。
- provider 列表：读状态库（经既有 `state_snapshot` 缝隙或等价引擎访问），
  而非 TOML `[provider.*]`——admin API 的运行期增删改落状态库，TOML 视图会
  过期（spec 要求"运行期变更免重启可见"）。
- 备选"只解析配置"被否：满足不了运行期场景；备选"经 gateway admin API
  活取"被否：依赖 `SEBAS_CONTROL_SECRET` + listen 可达，detached 常缺 secret。
- 状态库不可用时：gateway 段带可辨识的不可用指示（如
  `providers_available: false`），不冒充空集。

### D2：新增 `SessionRejection::BackendUnavailable { backend, cause }`

- Display："执行体不可用: {backend} — {cause}"；`Unavailable` 保留给真正的
  通道/权威不可达。native 缺凭据的拒绝在 `DualSessionBackend` 改投新变体。
- serde 兼容：新变体为 additive；新旧二进制同船发布，不存在跨版本解码
  场景。序列化形状跟随既有 tagged 写法，单测覆盖往返。

### D3：未知 `backend` 提示在 `DualSessionBackend::spawn` 单点校验

- 已知集合：缺省（None）→ acp；`native` → native；`acp` 与 `acp:<kind>` →
  ACP（沿用既有前缀路由）；其余 → typed rejection（复用 D2 新变体的形状，
  backend=原值、cause=unknown backend hint），不创建会话。
- 单点选在 DualSessionBackend 而非通道 server：channel server 的 Spawn 本就
  委托它，in-process 路径同样被覆盖，一处校验两形态生效。

## Risks / Trade-offs

- [state_snapshot 缝隙若不含 provider 视图] → 实现时若发现 key 缺失，在
  backend trait 加只读方法而非绕过缝隙直连引擎；改动限制在
  sebas-webui crate 内。
- [文案变更对已有 UI 断言/测试的扰动] → 只新增变体不改既有变体文案；
  前端对拒绝文案是透传展示，无匹配逻辑。
- [与在途变更同文件（agent_backend.rs / session_backend.rs）] → 本变更
  改动行集中在 GatewayInfo 装配、rejection 枚举、spawn 校验三处，先 rebase
  在途分支再动手。

## Migration Plan

纯 additive、单 commit 可回滚。无数据迁移、无配置变更、无协议破坏。

## Open Questions

（无）
