## Why

Models 分区现在既难用、方向也错。模型只能是一个字符串列表，一个 provider 装不下多个
模型，也无处标注模型能力（文字、视觉…）；预制 provider 只能改 API key 与默认模型，定制
provider 却要面对 `api_key_env` 与三个 URL 槽，输入负担过重。更致命的是编辑器
`<wa-dialog>` 把内部 `<wa-select>` 冒泡的 `wa-hide` 当作自身关闭事件，「新建（预制/
定制）」一碰选择框整个弹窗就退出，功能实际不可用。同时 Models 顶部的 Router 网关卡把
router 运行状态混进了 provider 管理，而 Services 分区本就是受管子进程状态的归属地。

## What Changes

- **BREAKING**：模型条目不再是字符串，改为结构化的条目（id + 能力标记）。能力标记词表为
  `text`（隐含，不落盘）/ `vision` / `audio` / `video`；旧的字符串条目在读取时接受并
  归一化，缺省即 `text`。
- provider 表单支持任意增删模型条目，每个条目可勾选能力。
- 预制 provider 表单收敛为：选 provider、填 API key、维护模型条目。实例名默认取 preset 名。
- 定制 provider 表单 = 预制 + provider 名 + base_url + 协议；其余 URL 槽、模型改名映射、
  `api_key_env` 收进默认折叠的「高级」。
- `api_key_env` 不再是可输入项；preset 自带的 env 名退化为无明文 key 时的隐式回退。
- 修复设置弹窗内所有 `<wa-dialog>` 的 `wa-hide` 误关闭（仅当事件源是对话框自身时才关），
  子 `<wa-select>` 收起不再连带关闭编辑器。
- Models 分区删除 Router 网关卡，不再展示任何 router 运行状态；router 状态由 Services 分区
  的受管服务行承载。
- 表单提供模型列表抓取入口；抓取机制与语义归 `add-fetch-models`，本 change 只消费它。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `webui`：`Provider management page` 表单语义重做（结构化模型条目 + 能力、预制/定制最小
  输入、折叠高级、选择框不再关闭弹窗）；`Services 分区数据源` 把 router 状态从 Models 移入
  Services，解除「Models 分区承载 router 总览」的约束。
- `provider-management`：新增「模型条目携带能力标记」——模型列表是条目列表而非字符串列表，
  词表与旧数据兼容规则一并定义。

## Impact

Rust：`sebas-router/src/config.rs`（`ProviderConfig.models` 改为条目列表、能力标记内嵌在条目上、
旧字符串兼容反序列化）、`sebas-router/src/models.rs`（静态表按 id 提供能力）、
静态 preset 表改为携带能力、core 状态库中 provider 行的形状（读取兼容、写入归一）。
前端：`views/settings-modal.ts`（表单重做、`wa-hide` 守卫、删除网关卡、能力勾选、抓取
入口）、`api/client.ts`（类型）。测试：frontend `settings-modal.test.ts`（含选择框回归）、
Playwright `settings.spec.ts` / `models.spec.ts`。

**BREAKING**：provider 的模型列表线上形状由字符串数组变为条目数组（读取兼容旧数据）。

## Non-goals

- 不做 provider 数据所有权的搬迁（`make-core-own-provider-data`）。
- 不做上游抓取的机制与端点（`add-fetch-models`）。
- 能力标记不参与路由或会话校验，不做「text-only 模型拒绝图片」这类强制。
- 不改飞书 `/provider` 卡片的界面布局。
- 不处理既有 spec 树重复与 `agent-workbench` 的 validate 失败。
