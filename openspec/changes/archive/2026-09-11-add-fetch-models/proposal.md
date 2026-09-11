## Why

预制 provider 的模型列表只能来自代码表，操作员看不到上游实际提供哪些模型。现有
probe 只在 router 进程上、且对 preset 派生的 provider 拒绝写回；「从官方 base url
抓最新模型列表」这件事既没对预制开放，也没有落在 core。按新契约 router 不再持有
provider 写路径，抓取必须成为 core 的能力，并且必须如实说明上游只给模型名、不给
上下文窗口等参数。

## What Changes

- core 提供模型列表抓取：用该 provider 解析后的 base url 与密钥向上游发一次只读
  `GET /models`（OpenAI 家族优先，Anthropic `/v1/models` 兜底），返回模型 id 列表；
  密钥不出现在响应与错误里，失败回 typed reason。
- 抓取对预制与定制 provider 都可用（预制用其代码表里的 base url）。
- **如实上限**：上游只提供 id；上下文窗口等参数仍由本地静态表按 id 解析，未知 id
  回落默认值并明确标注为未知，不从模型名猜。
- 抓取本身不改 provider 的任何字段：它只把列表呈现给操作员，由操作员挑选后再写入
  模型列表（那是编辑动作）。
- WebUI 的 provider 表单提供抓取入口与结果列表；没有可用 base url 的 provider 不
  渲染该入口。
- 飞书 `/provider` 卡片的探测改由 core 承载，行为语义不变（对 preset 派生仍是只读
  呈现，只改默认模型）。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `core-session-channel`：新增模型列表抓取能力——core 以 provider 的 base url 与密钥
  执行一次上游只读 GET，返回 id 列表，失败回 typed rejection。
- `provider-management`：`Model probing` 的承载从 router 改为 core，明确对预制可用，
  并写明「上游只给 id、参数由本地表解析」的如实上限。
- `webui`：新增从官方 base url 抓取模型列表的表单入口与结果呈现需求。

## Impact

Rust：`src/core_channel/`（provider 域新增抓取 op）、`sebas-router/src/admin.rs` 的
`fetch_models` 抽取为 core 可复用实现、`sebas-webui/src/routes.rs`（抓取端点改由
core 承载）、`router_client.rs`。前端：`api/client.ts`、`views/settings-modal.ts`
（抓取按钮与结果列表）。测试：`tests/state_*`、`sebas-router/tests/admin_test.rs`
（probe 用例迁往 core 侧）、Playwright `tests/testsuite-webui/tests/models.spec.ts`。

**BREAKING**：`/admin/providers/{name}/probe` 已由 `make-core-own-provider-data`
下线，抓取改走 core 面。

## Non-goals

- 不改模型条目的数据结构与能力标注（见 `redesign-provider-models-settings`）。
- 不从上游推导上下文窗口或能力标记：上游不返回这些字段，写进去就是编造。
- 不做定时或后台自动刷新，只在操作员显式触发时抓取。
- 不修改预制代码表的内容，也不因抓取结果改写预制的其它字段。
