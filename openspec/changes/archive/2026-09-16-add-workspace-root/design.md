## Context

项目目录面现状：`[watchdog.webui] allowed_roots`（可选多根白名单，config-only）只在注册时刻和 browse-dirs 显式 root 上执法；`/api/projects` 列表、会话查看/打开不复查路径；远端注册由节点自判存在性。存储侧项目路径已 canonical 化落库，`within_allowed_roots`（fs.rs）已有「canonicalize + 逐分量前缀 + fail-closed」的判定原语可复用。装配入口有两处（`webui_cmd.rs` 独立 webui、`run.rs` core 内嵌 webui），节点侧 `sebas-node` 有独立配置（`NodeConfig`，含 `default_work_dir`）。

## Goals / Non-Goals

**Goals:**
- 单一、恒存在的 workspace root 边界（env > config > cwd 回退 + 告警），主控与节点各管各的。
- 本机项目面四处执法：注册、列表、查看/打开、浏览。
- 远端注册经 `CheckPath` 携带节点侧 containment 判定。

**Non-Goals:**
- 不约束 ACP 子进程的文件访问；不在 core/dispatch 层二次执法（见 proposal Non-goals）。
- 不迁移或删除越界历史数据（隐藏 + 拒绝即可）。
- 不提供多根。

## Decisions

**D1 · 配置归属：控制平面顶层 `[workspace] root`，节点 `NodeConfig.workspace_root`，env 统一 `SEBAS_WORKSPACE_ROOT`。**
workspace root 是机器级概念：webui 只是执法者之一，节点没有 `[watchdog.webui]` 节。放顶层使「这台机器的项目根」只配一次，未来其他角色可复用。env 名与既有 `SEBAS_*` 约定一致，每台机器各自设置即天然满足「各节点互不影响」。
备选：`[watchdog.webui] workspace_root` —— 弃，机器级概念挂在 webui 节下语义错位，且节点无处安放。

**D2 · `allowed_roots` 与 `work_root` 一并删除，浏览起点合并进 workspace root。**
`WebUiState.allowed_roots: Vec<PathBuf>` → `workspace_root: PathBuf`（恒有值）；`work_root`（browse 默认起点，取 work_dir/cwd）随之删除——它唯一用途是 browse 起点，边界恒存在后再留两个「根」只会困惑（起点在边界外会展示一堆注册不了的目录）。`webui_allowed_roots()`、`within_allowed_roots`、`build_router_with_allowed_roots` 同批退役，判定原语改形为 `within_workspace_root(candidate, root)`。`safe_path` 的 `allowed_roots` 参数换成 workspace root。
兼容性依据：`WatchdogWebUiConfig` 未标 `deny_unknown_fields`，旧键在新二进制上被静默忽略，不炸解析；顶层 `Config` 同样未标，新键 `[workspace]` 对旧二进制也是静默忽略——双向滚动升级都不碎。

**D3 · 执法点与失败形态。**
- 注册（`projects_add` 本地分支）：越界 → 400「路径超出允许范围」，文案不回显服务端解析路径（沿用既有防泄露姿态）；越界判定先行于存在性判定（不借 400 文案差异探测目录存在性）。
- 列表（`projects_list`）：存储路径已是 canonical，与 canonical 后的 root 做逐分量前缀比较即可，不 re-canonicalize 存储值；root 解析失败 → 本机项目全部隐藏 + warn（fail-closed）。
- 查看/打开：session API 面解析会话绑定的本机 project dir，越界时 detail / message / switch / 携该项目的 create 一律 4xx typed 拒绝；close / archive 放行——围栏不该把垃圾锁死在里面。
- branch probe：越界项目按不可达处理，不暴露存在性。
- 远端（`projects_add_remote`）：消费 `PathCheck` 的新字段，`within_workspace = false` → 拒绝；字段缺省（老节点）→ 放行。

**D4 · 协议扩展：`SessionResult::PathChecked` 增 `within_workspace: bool`，serde 缺省 `true`。**
一次 `CheckPath` 往返已经覆盖注册判定，捎带一个字段最省；独立新 op 需要能力协商与额外往返，不值。serde default 让新主控读老节点的应答不炸（视为通过），与 spec「pre-containment answer stays compatible」一致。节点侧在 `CheckPath` handler 里对候选与自身 root 做 canonicalize 比较，缺 root（cwd 回退）同样先解析再比较。

**D5 · cwd 回退告警在装配处打。**
`webui_cmd.rs`、`run.rs`、`sebas-node` 启动装配三处：回退生效时 `warn!` 一条（含回退到的绝对路径 + 显式配置建议）。判定函数本身不打——它被高频调用，告警是启动期事实。

## Risks / Trade-offs

- [操作员升级后 cwd 成根，既有越界项目全部消失] → 启动告警明示当前生效根；部署文档给 `[workspace] root` 示例；越界会话仍可 close/archive 清理。这是用户明选的严格姿态。
- [cwd 回退在 daemon 部署下不稳（启动目录漂移换根）] → 文档强烈建议显式配置；告警常驻提示。
- [root 目录被删/移走 → 全部判越界] → 刻意 fail-closed；判定处 warn 日志留痕。
- [多根白名单用户（allowed_roots 配了多个目录）迁到单根要合并目录] → 符号链接把旧目录挂进新根即可；非目标里已声明不做多根。
- [老节点 containment 缺位留下远端出口] → 有意兼容；升级节点后自动收紧，spec 已写明该边界。

## Migration Plan

1. 先升级二进制（主控 + 各节点），旧键被忽略、新键尚不存在，行为等价于「cwd 回退 + 告警」。
2. 主控配置加 `[workspace] root`（或 env），重启后越界历史项目隐藏、可按需 close/archive。
3. 节点逐台配置各自 root 并重启；全部升级后远端 containment 完整生效。
4. 回滚：回退二进制；若配置里已有 `[workspace]`，旧二进制会忽略它（顶层无 deny），无需改配置。

## Open Questions

（无——四个分叉点已由用户裁定，其余为实现细节。）
