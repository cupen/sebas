## 1. provider 管理语义层(core 侧)

- [ ] 1.1 从 `src/provider.rs` 抽出与卡片渲染无关的语义函数(apply_preset_defaults、软删清默认、probe 端点推导 + 拉取),供飞书卡片与 webui 两条路径共用;重构不改行为,既有 `cargo test -p sebas` provider 相关单测全绿
- [ ] 1.2 `src/sebas_state` / `sebas_router` 侧核对 provider upsert/delete 的写入口(engine 路径),补 overlay 镜像同步断言:webui 路径写入后 providers.json 镜像与飞书路径写入结果一致;单测覆盖 rename 后镜像一致、删除默认 provider 清默认

## 2. 通道协议与核心分发

- [ ] 2.1 通道 provider 管理复用既有 `StateMutation { domain: "providers" }`(server.rs 已有 put/delete/save 域分发,gateway 侧已有 mutate_state client):`src/core_channel/server.rs` 的 `providers_mutation` 补 `set_default` op(`{op:"set_default", provider, model}` 更新 default_selection)与 `probe` op(经 `sebas_gateway` 的 fetch_models 拨号,可选 apply 写回 models);补单测(set_default 落盘、probe 失败不改目录)
- [ ] 2.2 Rust 集成测试覆盖:detached 形态经通道 StateMutation put 后 `state_snapshot("providers")` 反映变更、delete 触发软删+清默认、缺 name 的 put 返回 typed rejection 且无写入

## 3. native 后端模型来源

- [ ] 3.1 `src/agent_backend.rs`:`NativeAgentBackend` 装配时读 provider 状态(默认 provider 的 models catalog + default_model)推导 `available_models` / 初始模型;`SEBAS_AGENT_MODELS` / `SEBAS_AGENT_MODEL` 存在时整体覆盖;单测覆盖:provider 推导生效、env 覆盖优先、无 provider 无 env 时诚实降级
- [ ] 3.2 同步改写 wire-webui-sebas-agent-e2e 的 task 2.2 描述(`[agent] models` 静态配置 → provider 推导),保持该 change 任务清单与本变更一致;其 DualSessionBackend 分发部分不动

## 4. WebUI 服务端 API

- [ ] 4.1 `sebas-webui/src/session_backend.rs`:backend trait 增 provider 管理方法(list/upsert/delete/probe/set_defaults);in-process 实现直调语义层,`CoreChannelBackend` 实现走 2.1 消息;编解码与 fake-backend 单测
- [ ] 4.2 `sebas-webui/src/api.rs`:新增 `GET/POST/PUT/DELETE /api/providers`、`POST /api/providers/{id}/probe`、`PUT /api/agent-defaults`;list 响应只含 key-configured 布尔与掩码,不含明文 key;route 层单测用 fake backend 断言响应形状(有 key / 无 key / 真源不可用)与鉴权门控(非 loopback + auth 开启时未登录 401)

## 5. 前端 UI

- [ ] 5.1 `frontend/src/views/settings-modal.ts`:Models 分区重做为管理面——provider 卡片(名称、已配置状态、base URL、models 数、默认标记)、预设/自定义新增编辑表单(preset 推导回填 base URL)、probe / 删除 / 设默认控件、密钥 secret 掩码;前端单测覆盖卡片两种状态(已配置/未配置)渲染与表单提交载荷
- [ ] 5.2 `frontend/src/views/workbench-composer.ts`:模型下拉数据源改为 backend 级 available_models 优先,活跃会话 configOptions 仅对 ACP 会话生效;快照/事件刷新免 reload;单测覆盖:无会话时下拉有数据、ACP 会话用 agent 声明选项、目录变更后下拉更新

## 6. 端到端联调与收尾

- [ ] 6.1 双形态沙箱验收(release 构建,in-process 与 detached 各一遍):设置里新增 provider → 配 key → probe 出 catalog → 设默认;native 会话下拉出现 catalog 模型且选中生效;飞书卡片路径读同一份数据;`cargo test` 全绿
- [ ] 6.2 conventional commit 提交;核对 wire-webui-sebas-agent-e2e 任务清单已按 3.2 改写
