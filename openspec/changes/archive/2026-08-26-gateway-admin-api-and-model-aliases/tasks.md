## 0. router 侧：providers 拆回 providers.json

- [x] 0.1 `router/src/state_store.rs`：providers/deleted 段拆回 `~/.sebas/providers.json`（`SEBAS_GATEWAY_PROVIDER_OVERLAY` 可覆盖路径），state.json 只保留 mode/default_selection；一次性反向迁移：load 时发现 state.json 带 providers 段且 providers.json 缺失 → 搬出（tempfile+rename）→ 从 state.json 移除该段；迁移幂等可重入（providers.json 已存在则合并去重）；验证：单测覆盖「已迁移机器搬出且数据完整」「搬出后 state.json 不含 providers 段」「中途崩溃重入不丢数据」「老机器（providers.json 在）零动作」
- [x] 0.2 `router/src/crud.rs`（FileStore）与 `provider_card`/`spawn_env` 相关读路径回归验证：卡片 CRUD 与 Direct 模式 provider 解析全部改走 providers.json 后行为不变；验证：`cargo test -p router` + `cargo test`（spawn_env overlay 用例）全绿

## 1. 别名数据模型（gateway）

- [x] 1.1 `model_aliases` wire 结构（`{alias, provider, upstream_model?}`）+ 解析：`gateway/src/config.rs` 在 overlay 合并阶段读 providers.json 的 `model_aliases` 段，编译为「别名精确 RouteGroup（排在 config routes 之前）+ provider.model_map 插入 alias→upstream_model」；引用不存在 provider 的别名 drop + warn；验证：单测覆盖「alias 路由到绑定 provider」「alias 胜过同名 config route」「namespace 仍优先」「带 upstream_model 改写、缺省透传」「外部坏别名 drop 不启动失败」（对应 spec gateway-model-aliases 全部场景 + gateway-core Routing resolution order delta）
- [x] 1.2 校验辅助：把「候选 provider 条目在内存完整跑 resolve 管线」抽成可复用函数（preset 解析、URL 校验），供 admin 写路径复用；验证：单测断言无效候选（无 preset 无 URL）返回 Err 且错误信息含 provider 名

## 2. 可变内核重构

- [x] 2.1 `AppState` 拆分：路由相关字段（`providers`/`api_keys`/`table`/`auth_tokens`）收进 `Arc<RwLock<GatewayCore>>`，其余留外壳；`proxy::handle`/`require_key` 改读锁快照路径；验证：`cargo test -p gateway` 现有用例全绿（无行为变化的重构）
- [x] 2.2 热替换入口 `swap_core`：校验候选配置 → 写锁整体替换内核，返回错误时不动旧内核；`routing.rs` 的 `providers.get(...).expect(...)` 改为防御性 `NoRoute` 分支；验证：单测「swap 后新请求用新 table、swap 失败旧内核保持」

## 3. Admin API 面

- [x] 3.1 新建 `gateway/src/admin.rs`：admin router（`/admin/providers` CRUD、`/admin/model-aliases` CRUD、`/admin/providers/{name}/probe`、`/admin/reload`）+ `admin_auth` middleware（`SEBAS_CONTROL_SECRET` Bearer；无 secret 时 loopback-only + 启动 warn；401 message 通用串）；nest 进主 router（fallback 之上）；验证：集成测试「无 bearer 401」「错误 bearer 401 不回显」「无 secret 时 loopback 过、非 loopback 401」「/admin 路由不被 proxy fallback 吞」
- [x] 3.2 providers CRUD handler：GET 列表脱敏（`api_key_configured` bool，body 无 key 材料）；POST 创建（重名 409）；PUT 更新（空 api_key 保留旧值）；DELETE（config 种子来源的 provider 写墓碑）；全部走「校验 → Map 级 RMW（只动 providers/deleted 段，保留 model_aliases 等其它 key）→ tempfile+rename → swap_core」；验证：集成测试逐场景断言 + 「失败 400 时文件 mtime/内容不变」
- [x] 3.3 model-aliases CRUD handler：与 3.2 同款写路径（只动 `model_aliases` key）；别名校验（非空、无 `/`、provider 存在；重名 409、未知 404）；验证：集成测试覆盖 spec gateway-admin-api Model alias CRUD 场景
- [x] 3.4 probe endpoint：OpenAI `/models` 优先、Anthropic `/v1/models` 回退，上游 key 注入；`?apply=true` 时写回 provider `models` 字段；502 通用 message 不含 key；验证：测试起 mock upstream 断言列表返回与 apply 写回
- [x] 3.5 `POST /admin/reload`：手动触发重读+热替换，成功返回摘要、失败返回错误文本；验证：集成测试改文件后 reload 生效

## 4. 外部热重载

- [x] 4.1 引入 `notify` 依赖：watch providers.json 所在目录（路径过滤，规避 rename 换 inode），事件 300ms debounce 后走与 admin 写同一条 reload 管线；admin 自写用 `AtomicBool` 抑制一拍；notify 不可用降级 2s mtime 轮询；验证：单测/集成测试「外部写 providers.json（模拟卡片写入）→ debounce 后新 provider 可路由，无重启」
- [x] 4.2 失败保旧：reload 校验失败保留旧内核 + warn 日志 + `last_reload_error` 记入 admin 状态（供 stats）；下次有效写自动恢复；验证：集成测试「写坏 JSON → 旧路由继续服务 + stats 报 reload error → 写好文件后恢复」

## 5. Metrics

- [x] 5.1 新建 `gateway/src/metrics.rs`：计数器/直方图 registry（AtomicU64 per series，series 上限 1024 超出归并 `model="other"`）+ 观测点埋在 `settle_inner` 邻位与 auth/rate-limit 拒绝分支（requests_total、duration 直方图、tokens、rate_limited、upstream_errors、active_requests、start_time）；验证：单测「3 个请求后计数=3」「429 计入 rate_limited」
- [x] 5.2 `GET /metrics` 手写 Prometheus 文本输出，套 admin_auth；验证：集成测试「带 bearer 抓取得到合法文本格式 series」「无 secret 非 loopback 401」
- [x] 5.3 `GET /admin/stats` JSON：uptime、总量、per-provider 聚合（请求数/错误数/tokens/平均延迟）、last reload 状态；验证：集成测试「流量后 alpha 计数=3」「reload 失败后 stats 报错误」

## 6. webui BFF

- [x] 6.1 新建 `webui/src/gateway_client.rs`：reqwest 客户端（base `http://<gateway.listen>`、Bearer `SEBAS_CONTROL_SECRET`、3s 超时），providers/aliases/stats/reload/probe 的方法封装；验证：单测用 mock server 断言转发与超时降级
- [x] 6.2 `GET /gateway` 动态化：服务端经 client 拉取 providers+aliases+stats 渲染；gateway 不可达渲染降级卡片（保底显示启动快照 `GatewayInfo`）；验证：集成测试「改 provider 后刷新页面见新名（无 webui 重启）」「gateway 关闭时页面 200 + 降级提示」
- [x] 6.3 mutation 路由：`POST /gateway/api/providers`（create）、`/gateway/api/providers/{name}`（update）、`.../delete`、别名同款三动作、probe 动作——全部套既有 `admin_mutation_guard`（POST-only + origin check），无 secret 时 503；验证：集成测试「GET 打 mutation 路由 405」「非 loopback origin 403」「无 secret 503」
- [x] 6.4 模板改造 `webui/templates/gateway.html`：provider 列表 + 编辑表单（htmx 片段）、别名 CRUD 区、stats 数字卡片、reload 状态条；验证：手动/集成测试走通「新建 provider → 出现在列表 → 删除消失」全流程

## 7. 端到端与收尾

- [x] 7.1 扩展 `scripts/e2e_gateway.sh`（或新增 e2e）：起 gateway（无 secret，loopback）→ admin 建 provider+别名 → 请求经别名路由命中 mock upstream → 外部改 providers.json 热生效 → 抓 /metrics 断言计数；验证：脚本本地跑通
- [x] 7.2 全量回归：`cargo test --workspace` + `cargo clippy --workspace` 通过；`openspec validate gateway-admin-api-and-model-aliases --strict` 通过；验证：命令输出全绿
