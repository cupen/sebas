# Design — refactor-provider-data-model

## Context

Provider 配置当前是双槽位（`base_url_anthropic` / `base_url_openai`），协议由「哪个槽有值」隐式表达（`WireProtocol::{Anthropic, OpenAi}` 两档）；`/v1/responses` 只是 OpenAI 透传路径表中的一条，与 chat completions 共用 `base_url_openai`。preset 是 9 项 `&'static` 硬编码表（`sebas-router/src/config.rs` `PROVIDER_PRESETS`），resolve 时把 preset urls/models **复制**进 provider，显式字段可覆盖——数据一旦落盘就与代码脱钩。WebUI 的可编辑 provider 界面只存在于 preview 原型（前端硬编码 3 项 mock preset，与后端不同步）；正式 UI 只读。BFF 写端点 `/router/api/providers*` 已就绪且无前端调用方。

动到的接缝里「协议」有四个独立表示：`WireProtocol`（router 透传）、`AgentProtocol`（agent spawn，保持两档不动）、overlay Item 的自由字段 `protocol: auto|anthropic|openai`（Direct spawn 用，语义不变）、bot 表单协议快捷切换（写 `protocol` 字段，语义不变）。

## Goals / Non-Goals

见 proposal 的 Non-goals。设计层补充一条边界：**不引入协议转换**——responses 槽是纯透传第三通道，不是 OpenAI 两格式之间的翻译器。

## Decisions

### D1. `WireProtocol` 三档，而不是 openai 内部二选一

`WireProtocol::{Anthropic, OpenAiChat, OpenAiResponses}`，`url_for` 一对三。备选是「保持两档、在 OpenAI 档内按路径选槽」，被否：路由一致性检查、错误形状、admin 字段、`/admin/providers` 校验全都以协议为键，内部二选一会把特判散进每一层。三档让 sniff→route→upstream 全链路沿用既有模式：

- sniff：`/v1/responses`（含子路径）→ `OpenAiResponses`；其余 OpenAI 路径表 → `OpenAiChat`；anthropic 表与 header 仲裁不变；默认 `OpenAiChat`
- 鉴权：两个 OpenAI 档都注入 `Authorization: Bearer`（与 anthropic 的 `x-api-key` 区分不变）
- 错误形状：两个 OpenAI 档共用 `{"error":{...}}` 形状
- 前缀挂载 `/openai/*` 强制 `OpenAiChat`；`/v1/responses` 本身无歧义，不新增第三前缀

### D2. 三槽位字段与旧名拒绝

`ProviderConfig` / `RawProviderConfig` / preset 表 / admin JSON / `RouterInfo::ProviderInfo` 统一为 `base_url_anthropic` + `base_url_openai_chat` + `base_url_openai_responses`。旧 `base_url_openai` **不保留 alias**，且不能走 serde 静默忽略（否则违背「拒绝场景」）——在 raw 解析层对 provider 条目显式报错「`base_url_openai` 已移除，请改用 `base_url_openai_chat` / `base_url_openai_responses`」。校验规则同步：自定义 provider 至少配一槽；无槽即解析失败。

### D3. preset 跟随代码 = resolve 期物化 + 覆盖即报错

沿用现有「resolve 时从静态表物化」机制，但把语义拧紧：

- preset 派生条目允许携带的仅：`api_key` / `api_key_env`、`default_model`、`protocol`（Direct spawn 偏好）
- 携带任何 base_url 槽或 `models` → 解析/校验错误（bot 表单与 admin API 双入口一致拒绝）
- 删除 `resolve_preset_urls` 的「显式字段覆盖 preset」分支；单协议 preset 禁写对方端点的旧检查随之消亡（覆盖本身即非法）
- 代码更新 preset → 重启或 overlay 热重载后自动生效，存量条目零改动

`default_model` 与 `protocol` 归用户所有是澄清后的推论：代码表不定义它们，「除了 api key」指的是 preset 的数据（urls/models），用户的选择不与之冲突。probe 的 models 回写对 preset 派生条目改为不落盘（admin `?apply=true` 跳过、bot 结果卡仅展示/设默认模型）。

### D4. api_key 转正为 UI 一等字段

现状明文 `api_key` 仅测试用且 resolve 时 warn。WebUI 成为正式编辑入口后，overlay（本机 `providers.json`，回环 + secret 防护的 admin 面之外无人可写）是 key 的合理落点。决策：保留 `api_key_env` 优先级不变，去掉明文 key 的「仅测试」warn（降为 debug 日志）；展示仍走 `api_key_configured` 掩码。备选「UI 只允许填 env 名」被否：GUI 用户填 env 名体验荒谬。

### D5. preset 表暴露为只读端点 `GET /admin/presets`

WebUI 渲染 preset picker 和「跟随代码」只读值必须读运行中二进制的代码表，否则重蹈 preview 原型前后端 preset 不同步的覆辙。挂在 admin API（沿用 `SEBAS_CONTROL_SECRET` 鉴权），无 mutation 端点。webui BFF 侧 `/api/router` 的 `RouterInfo` 增加 `providers[].preset` 字段供列表区分 preset/自定义。

### D6. WebUI 前端：preview 原型移植进正式 settings，删除原型 provider 界面

以 `preview-app.ts` 的交互为蓝本在正式 views 落地 provider 管理页（列表/新建向导 preset|custom 双入口/编辑/删除/probe），数据源 `GET /api/router` + 写走既有 `/router/api/providers*` BFF（POST-only + origin check 已由 webui Mutation posture 覆盖）。按新语义裁剪：preset 表单无 url/models 输入，三槽仅 custom 表单出现；错误（409/400/503）页内呈现。原型里的 mock provider 管理 UI 一并删除，避免双真相。

### D7. bot 侧最小收敛（非交付面，语义必然后果）

`src/provider.rs` preset 表单去掉 models catalog 输入（urls 本就只读回填，改为不落盘、展示走 `/admin/presets` 同源逻辑）；`sebas-dispatch` 中对旧字段的引用更名编译修复；Direct spawn env 翻译的 OpenAI 分支取 chat 槽（Responses-only provider 对 Direct spawn 报错——agent 不说 responses 协议，`AgentProtocol` 不动）。不做任何 bot 新功能。

## Risks / Trade-offs

- [三档协议扩大状态空间，contract 测试矩阵 ×1.5] → 现有 contract_test 按协议参数化本就成熟，新增 responses 档用例与 chat 档同构
- [拒绝旧字段名会让手头未迁移的本地 config 启动失败] → 有意为之（fail fast 文化、未发布无存量）；报错文案给出替代字段名
- [providers.json 明文 key 落盘] → 文件已在本机用户目录；任务含设置 0600 权限（unix）；Windows 无等价强权限，记录为已知限制
- [preset 收敛删除覆盖能力，少数「改 preset 指向镜像站」诉求失去通道] → 用户已确认跟随代码语义；镜像站诉求走自定义 provider
- [`/admin/presets` 把 preset 表暴露给任何持 admin secret 者] → 只读、无敏感材料（urls/models 本就公开），风险可忽略

## Migration Plan

未发布，无数据迁移。一次性切换：代码更名 + `config/config.toml.example` 重写（顺带修掉其过时的 `protocol` 字段注释）+ 测试断言更新。回滚 = revert 提交。

## Open Questions

- 各 preset 的 responses 槽填什么：仅填供应商公开支持 Responses API 的端点（如 openai），其余留 `None`——实现时按文档核对，不猜。
