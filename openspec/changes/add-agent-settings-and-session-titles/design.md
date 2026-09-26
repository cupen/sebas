## Context

三个事实约束本设计（动机见 proposal）：

1. **命名管线已存在**：`fullSessionLabel` 优先链 `label ?? prompt_preview ?? session_id_short ?? key尾`（`sebas-webui/frontend/src/views/project-rail.ts:155`），`label`/`prompt_preview` 已全链路持久（domain → dispatch state → session_map 表 → DTO → WS 帧 → 归档）。缺的只是「往 label 写自动标题的生成器」与「聚焦头部漏网」（`dashboard.ts:1701` 裸显 `session_id.slice(0,12)`）。
2. **agent 目录闭装在 config**：`[acp.agents.*]`（`src/config.rs` AgentConfig，driver 标签封闭 `claude|acp`）启动时装进注册表（`src/run.rs:36-55`），UI 只读；`native` 是 `GET /api/agents` 硬编码追加行。settings.db 的 provider CRUD 是全套现成范本（`SchemaColumns/ActiveRecord` + `SETTINGS_TABLES` 注册 + state_store 领域 mutation + core_channel/session_backend 双接线 + routes.rs BFF + settings-modal 分区）。
3. **工作区无一次性 LLM 调用助手**：只有 sebas-agent 流式客户端与 sebas-router 探针（`probe.rs`，5s 超时、多形态 base_url 优先级）。

## Goals / Non-Goals

**Goals:**

- agents 定义落 settings.db（新 `agents` 表），UI 全权 CRUD，免重启生效
- config.toml `[acp.agents.*]` 平滑降级为种子源（真实实例升级无感）
- 首条消息异步生成标题，uuid 面归零；mode 标签瘦身

**Non-Goals:** 见 proposal Non-goals（不做 UI 写回 config、不做标题模型配置项、不做驱动实现扩展、不做 agent 市场）。

## Decisions

1. **agents 表落 settings.db + `agents` 领域 mutation，照 provider CRUD 范本双接线。**
   备选：settings KV 表存 JSON blob（否：无类型载体，违反持久层准入第 2 条，且无法按行 CRUD）；projects.db（否：agent 是全局能力配置，归属错误）；独立新 db（否：破坏 single-state-dir 分层）。表列镜像 AgentConfig 可承载字段：`id`(PK)、`driver`(`claude|acp`)、`path`、`args`(JSON)、`display`、`models`(JSON 可空)、`startup_timeout_secs`、`idle_kill_secs`、`work_dir`、`source`(`seed|ui`)、`created_at`/`updated_at`。claude 变体的 `sessions_dir` 不入表（UI 管理场景不需要，缺省走既有默认）。
2. **config = 种子源：启动时幂等导入 db 缺失 id，同 id store wins + 启动 notice。**
   备选：UI 写回 config.toml（否：双写者 + feishu app_secret 等敏感段误写风险）；硬拒绝 `[acp.agents.*]`（否：操作员真实实例升级即断粮）；config 永远赢（否：UI 编辑重启即失效，语义混乱）。种子导入放 config 装载后的接线处（`webui_cmd.rs`/`run.rs` 组装 sources 之前），fresh 状态目录（沙箱/CI）靠 config 播种的路子原样保留，e2e 菜谱不破。
3. **spawn 动态解析，免重启。** 会话 spawn 在 engine 侧（core 进程）解析 agent id：先查 config 注册表，miss 则读 agents 表构建等价 AgentConfig（claude → path+args，acp → command argv），两处皆无 → typed unknown-agent 拒绝。每次 spawn 直读（本地 SQLite，微秒级），不做缓存失效协议。
   备选：变更时重建启动注册表（否：需要广播失效机制，复杂度更高收益相同）。webui/detached 侧不解析（webui 不开库，只传 id 经 channel）——与「webui 无 DB 后端」现状一致。
4. **driver 标签仍封闭 `claude|acp`，opencode 是表单预设不是新驱动。** 表单三形态映射到存储：`claude` → driver=claude + path（缺省 "claude"）；`opencode` → driver=acp + command 预填 `["opencode","acp"]`（opencode-agent spec 既有形态）；自定义 ACP → driver=acp + 任意 argv。Windows 下 path/command[0] 沿用 `resolve_windows_executable`（npm 形态 CLI 解析已落地）。
5. **reachability 探测扩展到 store 行。** catalog 组装 = native 行 + agents 表全行，每行探测：claude → path、acp → command[0]，等价于 config 侧 `check_binary_reachable`；探测结果带 cause/version。`sebas agent-kinds list` 同源。
6. **标题生成器 = sebas-dispatch 内的小型 one-shot 模块。** 模式照抄 probe.rs：reqwest 直连 provider（`base_url_anthropic` 优先，`base_url_openai_chat` 兜底，凭据 `api_key`/`api_key_env` 取自 providers 域 default_selection），5s 超时、`max_tokens` 压小（64）、单轮 prompt（「为以下首条消息生成一个简短标题，只返回标题原文」）。不新增对 sebas-agent/sebas-router 的依赖。
   备选：复用 sebas-agent 流式客户端收集（否：引入重依赖且依赖方向存疑）；复用 router `/v1/messages` 代理（否：凭据在 provider store，绕道 router 徒增一跳）。
   失败语义：未配 provider / 网络 / 4xx5xx / 超时 → 静默放弃（preview 顶上），不重试、不告用户。
7. **auto-title 写进既有 `label` 字段，仅在 label 为空时落。** 触发：spawn 带 prompt 或 placeholder 首条消息提交后 `tokio::spawn`；写回前在 engine 内复检 label 仍为空（操作员已改名则丢弃标题，消除竞态）；标题清洗（换行折叠、≤40 codepoints 截断）。备选：新增独立 `auto_title` 字段（否：命名链、DTO、WS 帧、归档迁移四处都要加一层，收益为零——label 本就是命名链第一层，operator 覆盖语义现成）。操作员清除 label → 按既有命名规则回落 preview，不重新生成。
8. **聚焦头部修复收敛在命名链**：`dashboard.ts` 聚焦头部改调 `fullSessionLabel` 链（label/title/preview 均无时仍回落 id 截片——零信息场景保底）。/sessions 页 meta 单元格保留 id（元信息位、非标题，hover title 已有全文）。
9. **mode 文案纯前端**：`MODE_OPTIONS` 标签改 `Ask/Edit/Allow/Auto`；`new-session-dialog` 硬编码首选项改用 `MODE_DEFAULT_LABEL`（消除重复源）；wire 值与 `SESSION_MODES` 不动；dashboard 中文徽标不动。
10. **删除守卫**：删 agent 事务内查 projects 表 `default_agent` 引用并清除（projects 域既有 mutation 体系内加一个 op），已建会话不回收（agent 建会话后不可变，会话继续到自然结束）。

## Risks / Trade-offs

- [真实实例升级瞬间 agent 目录换源] → 种子导入幂等且 store wins，`[acp.agents.*]` 原样迁移成行；升级后 UI 可改，config 段删除与否均无害。
- [标题调用打真实上游花钱] → max_tokens 64、5s 超时、每会话一次、失败不重试；沙箱（debug test provider / 无 provider）自然走静默回退，套件不依赖外网。
- [per-spawn 读 agents 表] → 本地 SQLite 单行主键查，微秒级；写入频率极低，无并发热点。
- [schema 新表] → 走 sebas-db 非破坏 diff-sync（加列即可，无破坏步骤），与 settings.db 既有纪律一致。
- [UI 建 agent 打错路径] → catalog 探测给出 reachable=false + cause，创建会话表单禁用不可达项（既有行为），不新增失败模式。

## Migration Plan

1. settings.db 自动建 `agents` 表（启动 diff-sync，非破坏）。
2. 启动种子导入：config `[acp.agents.*]` → 缺失 id 建行（source=seed）；同 id 已存在 → 跳过 + notice。
3. 回滚：agents 表留置无害；恢复 config 权威需回退本次代码（无数据迁移要撤销）。

## Open Questions

无阻塞项。标题长度上限（40 codepoints）与 max_tokens（64）为初值，tasks 实现期可调，不影响 spec 行为。
