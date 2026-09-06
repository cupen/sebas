# 重构 provider 数据模型：三协议 base_url 槽位 + preset 跟随代码 + WebUI 管理界面

## Why

当前 provider 只有 `base_url_anthropic` / `base_url_openai` 两个槽位，`/v1/responses`（OpenAI Responses 协议）与 chat completions 共用同一槽位，无法指向不同网关；同时 9 个内置 preset 的归属语义含混——preset 派生 provider 既能被显式字段覆盖、又在表单里只读，用户实际可控的只有 API key。且 WebUI 正式的 provider 管理是只读列表，可编辑界面只存在于 preview 原型（mock 数据与后端不同步）。项目未发布，正是无兼容包袱的重构窗口。

## What Changes

- **BREAKING** provider base_url 拆为三槽位：`base_url_anthropic` / `base_url_openai_chat` / `base_url_openai_responses`，废弃统一槽 `base_url_openai`；`/v1/responses` 路由到独立槽，缺槽即 400 协议不匹配，不做回退
- **BREAKING** preset 语义收敛为「纯代码引用」：preset 派生 provider 的 base_url/models 一律跟随代码内置表解析（不落盘、不可覆盖），用户仅拥有 api key；显式覆盖 preset url 字段改为配置错误
- 自定义 provider 三槽位全自助（至少配一槽）
- WebUI 正式设置界面落地 provider 管理页（CRUD + preset 选择 + api key 编辑 + probe），消费已就绪的 `/router/api/providers` BFF 写端点；preset 派生 provider 的 url/models 展示为只读「跟随代码」
- router admin API、webui BFF `/api/router`、bot 卡片展示、config.toml.example 同步三槽位字段；bot 侧仅兼容性修改，不加新功能
- 不做任何旧格式迁移 shim（未发布）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `provider-management`：三槽位数据结构；preset「跟随代码」语义（禁止 url/models 覆盖）；CRUD 表单校验规则
- `router-core`：协议嗅探将 `/v1/responses` 归入独立协议档；槽位选择与缺槽 400；鉴权注入（两个 openai 档均 Bearer）
- `router-admin-api`：provider CRUD 请求/响应字段三槽位化
- `webui`：正式 provider 管理页需求（列表/创建/编辑/删除/probe/preset 只读展示）

## Non-goals

- 不改 agent→upstream 接缝（`AgentProtocol` 仍为 Anthropic/OpenAi 两档，agent 不说 responses 协议）
- 不做协议转换（router 保持纯透传）
- 不动飞书 bot 卡片的功能与交互（仅编译兼容与 preset 详情展示同步）
- 不做 preset 模板的用户级编辑/持久化（preset 表继续由代码硬编码）
- 不迁移任何旧配置格式

## Impact

- 代码：`sebas-router`（config/proto/routing/proxy/admin）、`sebas-webui`（api/models/routes + 前端 settings/preview 落地）、`src/provider.rs` 与 `sebas-dispatch`（字段更名编译兼容）、`config/config.toml.example`
- 测试：router contract 测试、e2e、验收套件中涉及双槽位断言的用例
- API：`/admin/providers*` 与 `/api/router` 的 JSON 字段变更（BREAKING）
