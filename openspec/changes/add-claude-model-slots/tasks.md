## 1. 域模型：ModelSlotMapping（Rust）

- [x] 1.1 `sebas-models/src/agent.rs`：定义 `ModelSlot`（fable/opus/sonnet/haiku，序即强弱）与 `ModelSlotMapping { slots, default_slot }`；`AgentRow.models` / `AgentDefinition.models` 类型从 `Vec<String>` 换为 `Option<ModelSlotMapping>`，JSON 反序列化对旧裸数组宽容降级为 `None`；`resolved_models()` 新语义（映射 → `default` + 启用档；无映射 → 内置表）。验证：单测覆盖新语义、旧形态降级、default 不在启用集被拒。
- [x] 1.2 `src/config.rs`：`AcpClaudeConfig.models` 接受结构化 TOML table（档位名键 + `default` 键），旧裸数组形态解析期报错且点名 `models` 键；`resolved_models` 与 store 侧同口径。验证：config 解析单测（合法映射 / 旧裸数组报错 / 缺 default 报错）。
- [x] 1.3 `sebas-dispatch/src/state_store.rs` agents 域 mutation：`models` 仅 claude driver 可携带，slot 名 ∈ 四档，启用集非空时 `default` 必须落在启用档；校验函数供 config 种子导入共享。验证：mutation 单测（合法映射、default 落在停用档被拒、无 default 被拒、非 claude driver 携带被拒、config 种子走同校验）。

## 2. spawn env 替换

- [x] 2.1 移除 provider 派生模型通道：删 `sebas-router/src/models.rs` 的 `map_to_env`/`ClaudeModelEnv`（先 grep 确认无其他消费者），删 `src/spawn_env.rs` 的模型 cover 派生与 `--model` argv 追加；provider 解析只留凭据半边。验证：`rtk cargo test -p sebas-router -p sebas` 相关旧用例按「无映射不注入任何模型键、任何模式无 `--model`」改写后全绿。
- [x] 2.2 新增 `slot_mapping_env(mapping)`：`ANTHROPIC_MODEL`=默认档值、各启用档 `ANTHROPIC_DEFAULT_{FABLE,OPUS,SONNET,HAIKU}_MODEL`、`CLAUDE_CODE_SUBAGENT_MODEL`=最弱启用档值，未启用档不出键；`resolve_spawn_overrides` 增加 agent mapping 输入，`session_boot.rs` 装配点从 `resolve_agent_config` 传入。验证：单测覆盖全映射/部分映射（停用档无键）/无映射零键/Router 模式与 Direct 同构/覆盖 OS 继承值。
- [x] 2.3 `sebas-acp/src/claude/driver.rs`：`builtin_claude_models()` 升级为 `default/fable/opus/sonnet/haiku`；`with_models` 注入链路吃 `resolved_models` 新语义。验证：driver 侧单测断言映射时词汇表为 `default`+启用档、无映射为内置五项。

## 3. WebUI 后端 wire

- [x] 3.1 `sebas-webui/src/api.rs`：`AgentLaunchPayload.models` 改 `Option<ModelSlotMapping>`；`/api/agents` 的 agent 信息面回填 models 原始映射（含 default），供前端表单预填与 new-session 联动。验证：webui 集成测试断言 POST/PUT 新形状、GET 回填、旧形状请求被 typed 拒绝。

## 4. 前端：统一目录与档位配置

- [x] 4.1 `client.ts` 类型更新；`settings-modal.ts` 的 `renderAgents` 重构为统一目录：一行一 agent（native/store 行/未装配方同列），行内安装状态徽标（内置/已安装+可达性/未安装），未装行安装按钮（沿用 install API）、已装行齿轮开编辑对话框，原配方独立区块删除。文案按现有 i18n 约定。验证：`pnpm run build` 通过 + testsuite-webui-sandbox 手动过列表形态。（浏览器旅程 `agents-slots.spec.ts` 首条已覆盖统一目录形态）
- [x] 4.2 claude 形态编辑表单加模型档位映射段：四档位行（启用 checkbox + 目录模型选择器 + default radio），选择器选项 = `loadModelCatalog` 平铺（provider 模型 + router 别名同权，无模式过滤），目录空时显示引导文案；`agentFormPayload` 组装新形状、编辑预填存量映射；非 claude 形态不渲染该段。验证：sandbox 手动保存/预填/校验回显 + 表单提交走 3.1 wire。
- [x] 4.3 `new-session-dialog.ts`：选中 claude agent 时模型下拉切为档位列表（`default` + 启用档，无映射用内置表），预选 agent 默认档并跳过「上次 (provider,model)」记忆规则；非 claude agent 维持 provider 目录与三级预选。验证：sandbox 手动切换 agent 观察下拉变化 + 建 claude 会话首回合模型正确。（浏览器旅程 `agents-slots.spec.ts` 第三条覆盖联动与预选）

## 5. 测试与回归

- [x] 5.1 进程级 e2e 旅程（`tests/testsuite_e2e_test.rs`）：配好映射的 claude agent 建会话 → 会话模型菜单断言 `default`+启用档 → 切档生效；未配映射 claude 会话菜单为内置五项。验证：`invoke testsuite-e2e --case <新旅程>` 通过。
- [x] 5.2 既有测试面清理：spawn_env / claude-env-cover / provider-management 相关断言按新行为改写（无 `--model`、无自动排位键）；webui/testsuite 内涉及 models 旧形状的用例更新。验证：`rtk cargo test` workspace 全绿。
- [x] 5.3 联调验收：`invoke testsuite-webui-sandbox --auth` 手动 GUI 旅程（Settings 配映射 → 建会话选档 → 会话内切档 → 重启进程映射仍在）；`invoke testsuite-e2e` 与 `invoke testsuite-acceptance` 全量通过。（新增浏览器旅程 `agents-slots.spec.ts` 三条全绿；`invoke testsuite-e2e` 95 通过；`invoke testsuite-acceptance` 退出码 0。**全量 webui 套件存在两类既有红**——native 装配 6 例与未读徽标 3 例，均经 `9c0a2c1`（pi 切换合并前）对照证实为 `f3936af` 引入的既有回归，非本 change 所致，详见 change 目录外的会话报告）
- [x] 5.4 文档：`config/config.toml.example` 补 `[acp.agents.<id>]` models 结构化示例；AGENTS.md 沙箱菜谱补 claude 档位映射一句（新形态与旧裸数组报错口径）。验证：示例 config 可被当前解析器接受。
