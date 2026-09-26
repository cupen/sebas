## Why

会话标题目前常以裸 uuid / 标识符出现（自动命名只落到首条消息预览，聚焦头部仍直显 `session_id` 截片），且没有任何「像样的标题」生成机制；同时 agent 目录只能改 config.toml 再重启，Settings 里无法管理。三条 webui 体验债一并偿还。

## What Changes

- **会话自动标题**：首条用户消息触发、异步调用 LLM 总结标题写入既有 `label`（模型取 providers 域 `default_selection`；失败/未配 provider 静默回退为首条消息预览）；operator 手改过的 label 永不被自动覆盖；工作台聚焦头部不再裸显 `session_id` 截片，改走统一命名链。
- **mode 选择框文案**：选项标签首字母大写并移除中文注释（`Ask` / `Edit` / `Allow` / `Auto`），默认项同步；wire 值 `ask|edit|allow|auto` 不变（纯实现，无 spec 增量）。
- **Settings agent 管理**：settings.db 新增 `agents` 表为 agent 目录唯一运行时权威；设置弹窗新增 `Agents` 分区，builtIn（`native`，即 sebas agent）恒在不可删，其余条目全量增删改；新建表单驱动形态 `claude` / `opencode`（command 预填）/ 自定义 ACP。config.toml 的 `[acp.agents.*]` **BREAKING** 降级为种子源：启动时幂等导入 db 缺失 id（同 id db wins），之后 UI 全权管理。agent 增删改免重启即时生效（spawn 动态解析）；删除被设为项目默认的 agent 时清除该默认。

## Capabilities

### New Capabilities

- `agent-settings`: settings.db 承载的 agent 目录管理——agents 表与领域 mutation、config 种子导入、设置弹窗 Agents 分区 CRUD、spawn 时动态解析与 reachability 探测、项目默认 agent 的删除守卫。

### Modified Capabilities

- `project-session-actions`: 「Session rows are named by the first prompt」命名链扩展——自动标题层插入优先级（operator label > 自动标题 > 首条消息预览 > 标识符），命名链覆盖聚焦头部；新增「首条消息异步自动标题」requirement。
- `webui`: 「HTTP route surface」agent 目录来源改为 native + settings.db 行并新增 agents CRUD 端点；「设置弹窗分区与缺省首项」分区表加入 `Agents`。
- `agent-driver`: 「Open agent registry keyed by kind, not a closed enum」——注册表从启动闭装扩展为 spawn 时动态解析（config 注册表 miss → agents 域快照）。
- `state-store`: 「State methods on the core channel」快照/变更域加入 `agents`。

## Impact

- Rust：`sebas-models`（AgentRow）、`src/sebas_state`（SETTINGS_TABLES 注册）、`sebas-dispatch`（agents 域 mutation、spawn 动态解析、标题生成器）、`src/core_channel/server.rs` + `sebas-webui/src/session_backend.rs`（域接线）、`sebas-webui/src/routes.rs`（BFF CRUD）、`src/config.rs`（种子导入）。
- 前端：`settings-modal.ts` 新分区、`new-session-dialog`/`mode-vocabulary.ts` 文案、`dashboard.ts` 聚焦头部命名链。
- 兼容：`[acp.agents.*]` 语义变化（见 BREAKING）；旧状态目录首次启动完成种子导入，无感迁移。

## Non-goals

- 不做 UI 写回 config.toml（config 只降级为种子源，不出现第二个写者）。
- 不新增「标题模型」配置项，不支持用会话自身 agent 生成标题。
- 不改 mode 的 wire 值、权限映射与 dashboard 徽标中文文案。
- 不做第三方 agent 市场/发现；驱动实现仍封闭于 `claude` | `acp` 两标签。
- 不处理标题的多语言偏好（跟随首条消息语言）。
