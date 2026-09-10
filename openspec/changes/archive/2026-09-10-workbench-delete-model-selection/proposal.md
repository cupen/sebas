## Why

工作台侧栏已覆盖「加项目 / 归档会话」，但**删除**入口缺失：项目栏没有移除按钮（后端 `POST /api/projects/{path}/remove` 从未被前端调用），rail 里的会话行也没有删除入口（会话 close 只存在于独立的 `/sessions` 列表页，而非工作台主界面）。同时，rail 点「+」创建的 0-turn 占位会话在深链页打开后，webui 后端的 focused 指针并未跟随跳转，导致工作台 composer 停在创建模式——在占位会话里敲下的第一条消息被当成「再建一个新会话」，而不是发给 claude code，体感就是「发不出去」。最后，创建模式的模型下拉对 ACP（claude code）会话是空的（agent 在 spawn 前不暴露模型面），操作员无法在第一回合就选定模型。

## What Changes

- **项目删除**：项目行 hover 出现「移除」按钮，二次确认后调既有 `POST /api/projects/{path}/remove`；项目下仍存活会话按既有 spec 迁移到 Inbox（不杀会话）。
- **会话删除（rail）**：rail 会话行在归档按钮旁加「关闭/删除」按钮，复用 `POST /api/sessions/{key}/close` 的杀进程+移除语义；删除当前聚焦会话后 focus 指针清空、工作台回到空态。
- **占位会话 focus 同步（修复发不出消息的体感）**：`POST /api/sessions/{key}/switch` 语义补强——凡经 switch 或深链页访问聚焦某会话，focused 指针立即跟随；rail「+」创建占位会话后前端补一次 switch 调用，保证工作台 composer 进入跟随模式，首条消息经 `POST /message` 发给 claude code 而不是误建新会话。
- **创建模式模型选择对 ACP 生效**：`POST /api/sessions` 的 `model` 字段对 ACP 会话从「silent no-op」升级为「spawn 后、首条 prompt 前经 `session/set_config_option` 下发」；无模型面的 agent（claude code）保持既有「不渲染下拉」的诚实缺省不变。跟随模式的模型切换沿用既有 `set_session_model` 路径，本提案不改动。

## Capabilities

### New Capabilities

（无新增 capability——全部为既有能力的补全。）

### Modified Capabilities

- `agent-workbench`：新增「项目/会话删除入口在 rail」与「占位会话创建后 composer 立即可发消息」两条 requirement；修订「Composer promises only what the process can do」补 focus 同步语义。
- `webui`：HTTP route surface 段补充 switch 端点「访问即聚焦」的语义（深链页访问同步 focus 指针的既有行为显式化）。
- `project-session-actions`：新增 rail 侧项目移除与会话关闭的 requirement（该 spec 目前只有 add/archive，缺 delete）。

## Impact

- **前端**：`sebas-webui/frontend/src/views/project-rail.ts`（项目行/会话行按钮 + 确认弹窗）、`views/session-detail.ts`（深链访问触发 switch 的既有行为显式化）、`views/workbench-composer.ts`（跟随模式进入条件不变，但 focus 同步后跟随模式能正确激活）。
- **后端**：`sebas-webui/src/api.rs`（switch 端点文档化 focus 语义；create_session 的 ACP `model` 字段下发路径）；`src/agent_backend.rs`（ACP spawn_with 的 model 在 spawn 后下发 set_config_option）。
- **e2e**：`tests/testsuite-webui/tests/`（projects.spec 补 rail 删除入口、session-mgmt.spec 补 rail 会话删除、session-roundtrip.spec 补占位会话首条消息旅程、models.spec 补创建模式 ACP 模型下发）。
- **无新增依赖、无破坏性 API 变更**（既有端点语义补强，不删不改 wire 形状）。

## Non-goals

- 不引入会话「硬删除」（抹掉磁盘/状态库痕迹）——本提案的会话删除即既有 close 语义（杀子进程+移除映射），archive 到期清理另行覆盖。
- 不改归档/恢复语义与 retention 配置。
- 不给 claude code 这类无模型面的 agent 伪造模型下拉（诚实缺省保持）。
- 不动 native kernel 的模型选择路径（既有 spec 已覆盖）。
- 不做项目重命名、路径编辑等 registry 写操作扩展。
