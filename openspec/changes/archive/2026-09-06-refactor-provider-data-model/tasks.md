# Tasks — refactor-provider-data-model

## 1. router 数据结构与协议三档

- [x] 1.1 `sebas-router`：`WireProtocol` 增 `OpenAiResponses` 档，sniff 路径表拆分（`/v1/responses` 及子路径 → responses，其余 openai 路径 → chat），鉴权注入与错误形状按 design D1 覆盖两 OpenAI 档；跑 `cargo test -p sebas-router` 中 sniff/proto 相关用例通过
- [x] 1.2 `sebas-router` config：`ProviderConfig`/`RawProviderConfig` 三槽位更名（`base_url_openai` → `base_url_openai_chat` + `base_url_openai_responses`），旧字段名显式报错（含替代字段名提示），`url_for` 一对三；自定义 provider ≥1 槽校验保留；单测覆盖旧名拒绝、三槽独立解析
- [x] 1.3 preset 语义收敛：删除 `resolve_preset_urls` 覆盖分支，preset 派生条目携带 url/models 即解析错误，允许字段仅 `api_key`/`api_key_env`/`default_model`/`protocol`；单测覆盖「派生条目跟随代码」「覆盖即报错」「key 不受 preset 更新影响」
- [x] 1.4 明文 `api_key` 去「仅测试」warn（降 debug），`api_key_env` 优先级不变；单测断言 warn 不再出现、优先级行为不变
- [x] 1.5 内置 preset 表迁移为三槽：按供应商公开文档为真实支持 Responses API 的 preset（如 openai）填 responses 槽，其余 `None`；顺带核对 9 项 preset 的既有 urls 未漂移；单测断言 openai preset 双 OpenAI 槽非空

## 2. admin API 与路由行为

- [x] 2.1 `GET /admin/presets` 只读端点（代码表直出：name + 三槽 + models），无 mutation 路由；集成测试断言响应随代码表、鉴权沿用 admin secret
- [x] 2.2 provider CRUD 请求/响应三槽位化并带 `preset` 字段：list 掩码不变、preset 派生条目 url 字段 400、custom 无槽 400、空 key 保留、409/404 语义不变；更新 `router-admin` 集成测试
- [x] 2.3 probe 端点：OpenAI 尝试顺序 chat 槽 → responses 槽 → anthropic 回退；`?apply=true` 对 preset 派生条目不落盘 models；集成测试覆盖「apply 跳过 preset 派生」
- [x] 2.4 路由一致性检查走三槽 `url_for`，缺槽 400 场景（含「responses 请求打到 chat-only provider 不转发」）；contract 测试补 responses 档透传/SSE/错误形状用例
- [x] 2.5 `--debug` 内置 `test` provider 覆盖三档 echo；跑 `cargo test --test contract_test` 相关用例

## 3. spawn 侧与 bot 最小收敛

- [x] 3.1 Direct spawn env 翻译：OpenAI 分支取 `base_url_openai_chat`，Responses-only provider 报解析错误；`protocol` 字段语义不变；单测覆盖 chat 槽取值与 responses-only 报错
- [x] 3.2 `src/provider.rs` / `sebas-dispatch`：preset 表单去 models catalog 输入、urls 只读展示改为不落盘（渲染源与 `/admin/presets` 同源）；全仓旧字段引用更名至编译通过；`cargo build` 无错误
- [x] 3.3 bot 卡片 preset 详情面板展示三槽；不加任何新交互；卡片渲染经 provider_card 单测核对（render_main_card 系列）；真实飞书端到端流程需 im-service + 飞书凭据，沙箱不可验（如实记录）

## 4. WebUI 正式 provider 管理页

- [x] 4.1 BFF/models 更新：`RouterInfo::ProviderInfo` 三槽 + `preset` 字段透出；`/api/router` 集成测试断言新字段
- [x] 4.2 前端 provider 管理页落地（列表/新建 preset|custom 双入口/编辑/删除/probe），写走 `/router/api/providers*`；preset 派生条目 urls/models 只读并标注「跟随代码」，custom 三槽可编辑，secret 不回填、空提交保 key，409/400/503 页内呈现；`pnpm build` 通过
- [x] 4.3 删除 preview 原型（整目录无任何引用，连同 mock provider 管理 UI 与 `PRESETS` 数组一并移除）；grep 确认仅剩历史注释
- [x] 4.4 沙箱联调：fake-claude 沙箱起 core+router+webui，HTTP 层验证 presets/CRUD/校验 400/409/删除/probe/三协议路由与缺槽 400；浏览器 GUI 验证列表渲染、New(preset) 编辑器（跟随代码只读面板）、GUI 创建 kimi-ui、GUI 删除。另修复两处暴露的集成缺口：core --router 实际端口回写 webui BFF 快照、debug test provider 在 admin 热替换后重注入

## 5. 收尾

- [x] 5.1 `config/config.toml.example` 重写为三槽位示例并清除过时 `protocol` 注释；示例仅注释变更，活跃配置段未动（root config 解析路径不受影响）
- [x] 5.2 全量验证：`cargo test --workspace`（唯一失败为 sebas-agent 17 个基线失败，干净树复现、与本 change 无关）、`cargo test --test core_flow_e2e_test --test acceptance_suite_test -- --ignored`（11/11 通过）、前端 vitest 109/109
- [x] 5.3 更新 `config/config.toml.example` provider 段落（README/部署文档未提及 base_url 字段名，无需改）；`openspec validate --strict` 通过
