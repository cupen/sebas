# Design: add-webui-allowed-roots

## Context

browse-dirs 的范围由 `fs::safe_path(path, explicit_root, server_default_root)`
单点裁决：显式 `root` 参数可完全替换服务端默认根，且对 root 本身没有约束
（任意存在目录都接受）。项目注册 `projects::add` 只做存在性 + 目录检查。
两个入口都缺「范围」概念。配置侧 `[watchdog.webui]` 现有 `auth` 开关，
work_root 由 `run.rs` / `webui_cmd.rs` 两处接线注入 `WebUiState`。

## Goals / Non-Goals

**Goals:**
- 一个配置项圈住 browse-dirs 的显式 `root` 与项目注册两条路径。
- 未配置时行为与现状完全一致（opt-in，不破坏现有部署与测试）。

**Non-Goals:**
- 见 proposal Non-goals：不改 picker 交互、不做热更新、不管会话运行时
  work dir、不管其它通道的路径来源。

## Decisions

**D1：白名单落在 `[watchdog.webui] allowed_roots`（`Vec<String>`，`~` 展开）。**
理由：范围是 webui 的安全姿态，与 `auth` 开关同区；`Vec<String>` 允许多根。
备选：`[fs] allowed_roots` 独立节——但 webui 是唯一消费方，不值得新顶层节。

**D2：白名单在 `WebUiState` 携带，`safe_path` 增加一个参数做校验。**
校验规则：显式 `root` canonicalize 后必须位于某个 allowed root 之下（或
等于其一）。默认根（work_dir/cwd 回退）在两处接线处自动追加进白名单，
保证「不带 root」的既有语义不变。备选：在 handler 层校验——但
`safe_path` 是路径语义单点（现有注释明确该定位），漏一层就会再出双入口
漏洞；注册校验也复用同一判定函数。

**D3：注册校验放在 `POST /api/projects` handler 内、`projects::add` 之前。**
理由：`projects.rs` 是纯注册表逻辑（有独立测试环境变量），塞范围语义会
污染其职责；handler 已持有 `WebUiState`。判定复用 D2 的同一函数
（入参为「路径是否在白名单内」，browse 与 register 共享）。
注意：越界但存在的路径在 canonicalize 前也可先做前缀预检并直接拒绝，
避免借错误信息探测目录存在性；canonicalize 失败的路径统一按越界处理
（不存在 → 现有的「目录不存在」400 优先级保持：先范围后存在性，避免
用范围错误反推路径存在与否——顺序为：范围判定 fail-closed）。

**D4：空白名单 = 不启用**（`Vec::empty` 与缺字段同义）。显式配置空数组
也视为未启用，避免「配置了但忘了填」把实例锁死的运维事故。

**D5：错误信息沿用现有中文风格，不回显服务端解析路径**（与 fs.rs 现有
防泄露注释同一姿态）。

**D6：错误处理基线——「服务端不可达」的统一前端表现。**
白名单 400 属服务端业务错误，经由 D5 的现有错误链路透出即可；但本变更
前端验收还要确认整个降级链路在故障下表现正确。约定三层：

1. **连接层（全局）**：`/ws` 断线期间在 app-shell 顶部渲染全局连接横幅
   （「与服务器的连接已断开，正在重连…」），重连成功即消失。现状是断线
   静默、仅重连后触发 `sebas:refetch`（`shared-ws.ts`），操作者对冻结
   视图无感知；`WsClient` 已有指数退避与 `onReconnect` 钩子，只需暴露
   连接状态。备选：每视图各自提示——重复实现且不覆盖共享连接这一事实，
   否。
2. **数据层（fetch）**：`ApiError` 语义保持「服务端 4xx/5xx + 后端 error
   文本」；网络级失败（`fetch` 抛 TypeError，无 HTTP 响应）统一包装为
   可识别的「服务不可达」错误类，视图据此区分「后端拒绝」与「进程没了」。
3. **视图层（局部）**：列表类视图（dashboard、project-rail、sessions）
   初始加载失败显示内联失败态 + 重试入口而非空白；folder-picker、
   transcript 沿用现有内联报错。

「core 挂了但 webui 进程活着」已有 reachability 门禁（composer 禁用提交），
维持不变；仅把 composer 中 summary 请求本身失败时的静默（现注释明确
「leave reachability null」）改为同款不可达提示，与 `reachability.ok =
false` 一致对待。

## Risks / Trade-offs

- [多根列表下用户误配相对路径] → 配置解析时对非绝对路径告警日志 +
  `~` 展开；相对路径按相对于进程 cwd 解析（与 work_dir 现语义一致）。
- [symlink 使 canonicalize 后根变化] → 白名单条目同样 canonicalize 后
  比较，与 safe_path 现有分量比较逻辑一致。
- [测试覆盖双接线形态] → 单测覆盖 `safe_path`/判定函数；接线处沿用
  testsuite-webui-sandbox 做一条 e2e 冒烟（越界 400）。
- [全局横幅与既有视图生命周期冲突（如 login 页）] → 横幅挂在 app-shell
  层；login 等无 ws 依赖视图下横幅同样无害（断线提示本身为真）。
- [网络级错误包装改变既有 catch 分支行为] → 包装只针对 fetch 抛出的
  TypeError，`ApiError` 的构造点与 `status` 字段不变；client 单测锁定
  两类错误的可区分性。

## Migration Plan

纯加法配置，未配置时零行为差异；回滚 = 删配置项。无数据迁移。

## Open Questions

（无——空白名单语义与默认根自动入列均已定；如运营中需要「完全锁死
不允许任何显式 root」，届时再议，不影响本变更。）
