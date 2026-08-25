# Design: gateway-admin-api-and-model-aliases

## Context

gateway 目前是「一次解析、终身只读」的进程：`GatewayConfig::parse` 在启动时合并 config.toml 种子 + overlay 文件，`AppState` 各字段 `Arc` 包死后不可换。三进程拓扑（watchdog → core / webui / gateway 子进程，共享 config 路径与 `SEBAS_CONTROL_SECRET` env）决定了 webui 编辑 gateway 只能走跨进程 HTTP。状态真源已迁移到 `~/.sebas/state.json` v2（router 侧 `state_store`，含 `providers`/`deleted`/`mode`/`default_selection`），而 gateway 还在读已废弃的 `providers.json` —— 断链见 proposal。webui 已有 htmx + admin 鉴权（密码/CSRF/origin check）+ POST-only mutation 姿态，且 `webui` 页面数据全部来自 `WebUiState` 启动快照。

## Goals / Non-Goals

**Goals:**

- admin 面与 metrics 挂在 gateway 同端口，复用现有 axum layer 栈，不新增监听
- 配置热生效：单进程内可原子换「路由内核」，在途请求不受影响
- providers.json 成为 core（卡片）与 gateway（admin API）双进程共享的协作文件，两写者共存
- webui 侧 secret 不出后端（BFF 模式），复用既有 mutation 姿态

**Non-Goals:**

- 不改飞书卡片写入路径（router crate 零行为变更，仅靠文件协作）
- 不做路由故障转移、时序图表、per-token 限流配置（proposal Non-goals）
- 不做 admin API 的多用户/审计日志体系（单 secret，够用）

## Decisions

### D1. 可变内核：`RwLock<GatewayCore>` 而非重建进程

`AppState` 拆为「不可变外壳 + 可换内核」：`cfg` 里的 `listen`/`max_body_bytes`/超时/`debug`/`usage_file` 等启动期字段留在外壳；路由相关字段（`providers`/`api_keys`/`table`/`auth_tokens`）收进 `Arc<RwLock<GatewayCore>>`。proxy handler 读锁取快照（clone Arc 引用，非深拷贝），admin/reload 写锁整体替换。

- 每请求一次 `RwLock::read()`：tokio 下用 `parking_lot` 风格的 `std::sync::RwLock`（临界区纯指针拷贝，纳秒级，不值得上 `tokio::sync::RwLock` 的 async 开销）
- 备选「reqwest client 也放进内核重建」被否：连接池热替换会打断 keep-alive；client 只依赖超时配置，留在不可变外壳
- `auth_tokens` 放内核（providers.json 不含它，但外部 reload 语义统一走「重读全部配置」）

### D2. 别名编译进现有路由积木，不新增解析分支

`model_aliases` 在配置解析期编译为：每别名一条精确 `RouteGroup{model: alias, providers: [provider]}` + provider 的 `model_map` 插入 `alias -> upstream_model`。别名优先于 config.toml 精确路由靠**插入顺序**实现：编译后的别名 routes 排在 config routes 之前（`match_route` 顺序扫描精确段，先扫别名组）。命名空间优先级天然保留（`resolve` 先查 namespace）。别名与 config route 撞名时别名组在前即胜出，符合 spec。

- 备选「RouteTable 增加 alias 一等字段 + resolve 插入别名查找步骤」被否：路由引擎零改动的收益更大，且 glob 段语义不变（别名组不含 `*`，永不参与 glob 匹配）
- 编译点放在 `GatewayConfig::parse` 之后的 resolve 阶段（overlay 合并完、`RouteTable::from_config` 之前），别名引用不存在的 provider 在此 drop + warn（自愈）

### D3. providers.json 单一真源：router 拆回 + gateway 双写者

方向（与最初草案相反）：provider 数据回归独立的 `~/.sebas/providers.json`（`SEBAS_GATEWAY_PROVIDER_OVERLAY` 可覆盖），作为 providers / deletion tombstones / `model_aliases` 三段的单一真源。gateway 的现有读路径（`merge_provider_overlay`）本来就是它，断链的修复在 router 侧：`state_store` 把 providers/deleted 段从 state.json 拆回 providers.json，mode/default_selection 留 state.json；已在 2026-08-17 迁移过的机器（state.json 有 providers 段、providers.json 已被删）在 router 加载时一次性反向搬出。

读（gateway）：config.toml 种子 → providers.json overlay（`providers`/`deleted`/`model_aliases` 三段）。文件缺失 = 纯种子（今日行为）。

写（admin API）：**校验先行**——把请求折成候选配置，在内存里完整跑一遍 resolve 管线（preset 解析、URL 校验、别名编译），任何失败 400 拒绝、不碰文件；通过后以 `serde_json::Map` 读 providers.json → 只改写目标段（providers/deleted 或 model_aliases），文件内其它 key 原样保留 → tempfile + rename 原子落盘 → 触发内核热替换。

- 备选「gateway 改读 state.json（本 change 初稿方案）」被否（用户决策）：providers.json 与 gateway 现有读路径天然对齐，且 provider 数据独立成文件职责更清晰；代价是 router 要做一次反向拆分
- 备选「gateway 独立新文件」被否：webui 编辑的 provider 必须对卡片与 Direct 模式 spawn 路径立即可见，必须与 router 共用一个文件
- 三写者竞态（卡片 / admin API / 一次性迁移）：entry 级 RMW + 原子 rename 下最坏情况是同一条 provider 条目后写覆盖先写；无 torn read 可能。窗口毫秒级且操作低频，不做文件锁

### D4. 外部变更感知：notify + debounce，失败保旧

`notify` crate（inotify backend）watch providers.json 所在目录（watch 目录而非文件：rename 替换会换 inode，文件级 watch 会失联；目录事件按路径过滤）。事件 → 300ms debounce → 与 admin 写共用同一条「重读 + 校验 + 热替换」管线。校验失败：保留旧内核 + `last_reload_error` 记入 admin 状态 + warn 日志；下次有效写自动恢复。admin 自己的写会先 `AtomicBool` 抑制一拍（避免自己触发一次冗余 reload）。router 侧卡片写路径（state_store/FileStore）不感知 notify——它是被 watch 的一方。

- 备选「mtime 轮询」被否（用户已决策：notify 实时）
- inotify 在 macOS/其它平台的 fallback：notify 抽象层处理，测试走 `parse` 直接重读，不依赖真实事件

### D5. Admin 鉴权与路由装配

新 `gateway/src/admin.rs`：axum `Router` 挂 `/admin/*`（providers/aliases CRUD、probe、reload、stats JSON），`/metrics` 挂 `metrics.rs` 生成的 handler。两者套同一个 `admin_auth` middleware（D6 的鉴权），再整体 `nest` 进主 router —— 在 `proxy::handle` fallback **之上**，不受下游 `require_key`/`rate_limit` 影响（那两层继续只管透传流量）。`/healthz` 语义不动。

鉴权中间件：`SEBAS_CONTROL_SECRET` 非空 → 校验 `Authorization: Bearer`；为空 → 用 `ConnectInfo<SocketAddr>` 判 loopback 放行、否则 401，启动时 warn 一次。401 message 恒定通用串（与 `auth.rs` 同款铁律）。

### D6. Metrics：手写最小 registry，不引 prometheus crate

`metrics.rs`：`Arc<Mutex<...>>` 或分片原子计数（`AtomicU64` per label 组合，HashMap<String, AtomicU64>）+ 固定 bucket 直方图（10ms…10s 对数桶）。观测点插在 `proxy::handle` 的既有结算路径（`settle_inner` 旁边）+ auth/rate-limit 拒绝分支。文本格式手写输出（`# HELP/# TYPE` + series 行）。

- 备选 `prometheus` crate 被否：本 codebase 惯例是最小依赖（手写 glob、手写 SSE parser）；指标族少（7 个 family），格式简单稳定
- label 基数：`model` 标签取客户端原始 model 串，靠 `/admin/providers` 可见的 models 目录自然约束；不做截断（spec 未约束，超基数风险记录在 Risks）

### D7. webui BFF：后端代理 + POST-only 动作路由

webui 新增 `gateway_client.rs`（reqwest，base `http://<gateway.listen>`、Bearer `SEBAS_CONTROL_SECRET`、3s 超时）+ 路由：`GET /gateway`（服务端拉 stats+providers 渲染，gateway 不可达渲染降级卡片）、htmx 片段路由（provider 行/表单/别名列表）、`POST /gateway/api/providers`（create）、`POST /gateway/api/providers/{name}`（update）、`POST /gateway/api/providers/{name}/delete`、别名同款三动作 + probe。所有 mutation 走既有 `admin_mutation_guard`（POST-only + origin check），secret 只存在于 webui 进程内。`GatewayInfo` 启动快照字段保留（gateway 关闭时的兜底显示），页面主体改请求期拉取。

- 动作式 POST（非 REST PUT/DELETE）：与现有 mutation posture 一条规则管到底，htmx 表单直发
- gateway 不可达：503 JSON（API 路由）/降级 UI（页面路由），不重试不缓存

### D8. 剥离 `expect` 隐患顺带修复

`routing.rs` 的 `providers.get(&provider_name).expect(...)` 依赖「from_config 镜像保证存在」跨模块不变量。内核可热替换后该不变量仍成立（编译期别名 drop 保证了引用闭合），但改成返回 `RouteError::NoRoute` 的防御分支，成本一行，消除热重载时代的 panic 面。

## Risks / Trade-offs

- [providers.json 双写者竞态：卡片写 provider 与 admin 写 provider 同时发生，后写覆盖；迁移搬出也可能与在线写入交叠] → 窗口为毫秒级且操作低频（人速点击）；两侧 RMW 均保留对方 section，丢的最坏是单条 provider 条目，用户重放即可。不做锁。
- [model label 基数失控：恶意/异常客户端发随机 model 串撑爆 metrics 内存] → 计数器 map 设上限（如 1024 个 series，超出归并到 `model="other"`）；实现细节，不影响 spec。
- [notify 目录 watch 在某些 FS（NFS）不可用] → notify 不可用时降级 2s mtime 轮询（同一 reload 管线，检测层可插拔）。
- [热重载校验失败静默保旧，用户以为改成功了] → `/admin/stats` 暴露 `last_reload_error`；webui gateway 页顶部常驻 reload 状态条。
- [admin 面暴露在售卖场景的公网端口上，仅 secret 保护] → secret 由 watchdog 生成注入（不落配置文件）；standalone 无 secret 强制 loopback。暴力破解面与 `/healthz` 同级，可接受；后续可加限流（记录为后续工作）。
- [`SEBAS_CONTROL_SECRET` 泄露即全控 gateway 配置] → secret 本就是进程间信任边界（webui/core 同权）；文档明示不要把 secret 传给不受信环境。

## Migration Plan

1. **router 侧一次性拆回**：新版 `state_store::load()` 读到 state.json 带 providers/deleted 段（已迁移机器）且 providers.json 缺失时，把这两段写入 providers.json（tempfile + rename），随后从 state.json 移除该段并重写（state.json 只剩 mode/default_selection）。老机器（providers.json 本来就在）零动作。崩溃窗口：搬出后、state.json 清理前重启 → 下次 load 幂等重入（providers.json 已存在则合并去重）。
2. **gateway 侧**：读路径不变（providers.json）；新增 `model_aliases` 段解析。新 gateway 部署即生效，无数据迁移。
3. **回滚**：旧 router 读 providers.json（原始行为兼容）；旧 gateway 不解析 `model_aliases` 段（`#[serde(default)]` 忽略）。回滚安全，无需数据迁移。
4. **灰度**：单机单进程，无灰度需求。

## Open Questions

- probe 的 `?apply=true` 写回 provider `models` 字段后，是否要把 models 目录同步用于 `/v1/models` 列表响应（gateway 目前不实现 models 列表端点）——与 spec 无关，实现期顺手决定即可。
