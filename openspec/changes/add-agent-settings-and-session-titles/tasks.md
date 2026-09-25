## 1. agents 持久层（settings.db）

- [ ] 1.1 `sebas-models` 新增 `AgentRow`（`#[derive(SchemaColumns, ActiveRecord)]`，table="agents"，pk="id"；列集见 design 决策 1）并定义与 AgentConfig 等价载荷的互转函数；单测：row↔载荷往返、derive 生成的 save/find/all/delete 可用
- [ ] 1.2 `src/sebas_state/repo.rs` SETTINGS_TABLES 注册 agents 表（手写 DDL：PK/约束；columns 取 `AgentRow::schema_columns()`），确认 `StateWriter::start_settings` 无需改动；单测：fresh 状态目录启动建表、旧库 diff-sync 加表不破坏既有数据
- [ ] 1.3 `sebas-dispatch/src/state_store.rs` 新增 `agents_mutation`（ops：`put`/`delete`），删除时同事务清除 projects 表引用该 id 的 `default_agent`；单测：put→snapshot 可见、delete→行消失且项目默认被清、未知 op 报错
- [ ] 1.4 双接线：`src/core_channel/server.rs`（snapshot_domain 增 `agents` 分支、StateMutation 路由）与 `sebas-webui/src/session_backend.rs`（InProcessBackend 增同名分支）；单测：两条路径 snapshot/mutation 行为一致，unknown domain 报错文案不变

## 2. config 种子导入

- [ ] 2.1 启动接线处（`src/webui_cmd.rs` 与 `src/run.rs` 组装 agent sources 前）实现种子导入：config `[acp.agents.*]` 中 db 缺失的 id 建 `AgentRow`(source=seed)，同 id 已存在跳过并打 notice（指向 Settings）；单测：fresh 目录导入全量、二次启动幂等零变更、同 id 冲突 store wins
- [ ] 2.2 `config/config.toml.example` 与 AGENTS.md 沙箱菜谱补一段说明（agents 也可在 Settings 管理，config 段为种子）；验证：文档审读 + 示例配置仍可解析

## 3. spawn 动态解析与可达性

- [ ] 3.1 engine spawn 路径动态解析：config 注册表 miss → 读 agents 表构建等价 AgentConfig（claude→path+args，acp→command argv；Windows 经 `resolve_windows_executable`）；两处皆无 → typed unknown-agent 拒绝；单测：store-only agent 可 spawn（fake-claude stub）、未知 id typed 拒绝、config agent 行为不变
- [ ] 3.2 catalog 组装改 union：native 行 + agents 表全行，逐行探测（claude→path，acp→command[0]）带 reachable/cause/version；`sebas agent-kinds list` 同源；单测：native 恒在、store 行探测结果如实、不可达行带 cause

## 4. BFF CRUD 端点

- [ ] 4.1 `sebas-webui/src/routes.rs` 新增 `POST /api/agents`、`PUT /api/agents/{id}`、`DELETE /api/agents/{id}`（读改走既有 `state_snapshot/state_mutate("agents")`），错误映射复用 `map_mutation_error`；单测：CRUD 往返、core 不可达 503、非法 driver/payload 400、native id 拒绝写
- [ ] 4.2 `GET /api/agents` 响应保持 AgentKindInfo 形状（wire 上不出现 driver 字段）；单测：payload 断言无 driver/实现字段泄漏

## 5. 前端：Settings Agents 分区

- [ ] 5.1 `settings-modal.ts` 新增 `Agents` 分区（导航顺序 `…Models → Agents → Env Vars · About`）：native 行只读徽标、条目编辑/删除（删除二次确认）、新建表单三形态（claude/opencode 预填 `["opencode","acp"]`/自定义 ACP argv）；验证：沙箱 GUI 手测 + webui 套件既有 settings 分区测试更新
- [ ] 5.2 `new-session-dialog.ts` agent 下拉消费新 catalog（不可达项禁用 + cause 既有行为不变），UI 新增 agent 免重启可选；单测/套件：webui testsuite 新增 journey「UI 添加 agent → 立即建会话 → fake-claude 完成一回合」
- [ ] 5.3 前端 TS 类型与 `api/client.ts` 增补 agents CRUD 方法；验证：tsc 通过、既有 lint 零新增告警

## 6. 会话自动标题

- [ ] 6.1 `sebas-dispatch` 新增 one-shot 标题模块（模式照 probe.rs：reqwest 直连，`base_url_anthropic` 优先 / `base_url_openai_chat` 兜底，凭据取 default_selection，5s 超时，max_tokens=64，只返回标题原文）；单测：形状解析、无 provider/HTTP 失败/超时各返回 None
- [ ] 6.2 触发与写回：spawn 带 prompt 或 placeholder 首条消息后 `tokio::spawn` 异步生成；标题清洗（换行折叠、≤40 codepoints）；写回前 engine 内复检 label 仍空，写入走既有 set_label 路径（session.updated 帧天然下发）；单测：operator 先改名则标题丢弃、清洗截断、失败静默（preview 保持）
- [ ] 6.3 e2e/验收：fake-provider 或 debug test provider 下验证「首回合零延迟、标题就绪后列表与聚焦头部原地更新」；归档→恢复后标题仍在

## 7. 命名链修复与 mode 文案

- [ ] 7.1 `dashboard.ts` 聚焦头部改走 `fullSessionLabel` 链（label→title→preview→id 截片保底）；webui 套件断言：有 preview/label/title 时头部不出现裸 uuid
- [ ] 7.2 `mode-vocabulary.ts` 标签改 `Ask/Edit/Allow/Auto`，`MODE_DEFAULT_LABEL` 同步；`new-session-dialog.ts:377` 硬编码改用 `MODE_DEFAULT_LABEL`；webui 套件 mode 选择框断言更新（wire 值断言保持 ask/edit/allow/auto）

## 8. 收尾验证

- [ ] 8.1 全套回归：`invoke testsuite-webui`、`invoke testsuite-e2e`、`invoke testsuite-acceptance` 全绿（POSIX 专属豁免按既有清单记录）
- [ ] 8.2 沙箱真实升级路径演练：带 `[acp.agents.claude]` 的旧 config + 含既有会话的状态目录启动一次，确认种子导入、旧会话可达、Settings 可改；`cargo clippy` 工作区零新增告警
