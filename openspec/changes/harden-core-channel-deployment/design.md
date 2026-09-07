# Design — harden-core-channel-deployment

## Context

通道装配现状（通信拓扑见探索结论）：watchdog 启动时生成随机 secret（pid+纳秒），经 env `SEBAS_CORE_SECRET` 注入 core/webui/router/im 子进程；core 仅在 env 非空时武装通道（`src/run.rs:322`），客户端无条件建连（`src/webui_cmd.rs:195`）。secret 不落盘，协调完全靠"watchdog 亲缘"。watchdog 监督健康模型 = 进程活着 + stdout ready 行（`src/watchdog.rs:104`）；崩溃重启 1s / 冷却 30s / bind 失败(75)→Degraded（`src/watchdog/supervisor.rs:428`）。webui 侧：composer 局部轮询 reachability 门禁（`workbench-composer.ts:349`），全局横幅只有 ws 断连一种；项目注册通道不可达时静默降级本地文件并 201（`sebas-webui/src/api.rs:655`）。

## Goals / Non-Goals

**Goals：**
- 消灭"core 活着但通道永不武装"这一状态类（自动武装 + secret 发现）。
- 让 supervisor 的健康模型隐含通道契约（ready ⟹ 已武装），监督逻辑零改动。
- 核心不可达在 webui 全局可见；一切降级如实提示。

**Non-Goals：** 见 proposal（不做网络通道、不做密钥调度、不做 supervisor 深度探测、不改 webui 存活姿态）。

## Decisions

**D1 secret 文件位置：config 目录相对，而非 socket 路径旁。**
缺省 `<config 文件所在目录>/core.secret`，`[watchdog.core] secret_file` 可显式覆盖。备选：socket 路径旁——unix 上自然，但 Windows named pipe 没有文件系统目录，需要第二套回退逻辑；`~/.sebas/core.secret`——会落进真实 HOME，正是沙箱规则反复警告的坑。config 相对方案跨平台一致，且双进程读同一份 `-c` config，路径天然一致；沙箱用自己的 config，隔离天然成立。

**D2 secret 解析顺序与重读时机：env → 文件 → 空值告警；文件在每次连接尝试时读取。**
客户端把 secret 从构造期常量改为动态解析：env 存在则缓存（零开销），否则每次 connect 前读文件。收到 `secret rejected` 或断线重连时天然拿到新钥——core 重启换钥后运行中的 webui 自愈，无需通知机制。备选：文件 watch（inotify）推送——过度设计，退避重连已覆盖。

**D3 自动武装无 opt-out 开关。**
通道是本机 IPC、0600、双因子（secret + unix uid 校验），常开运维成本为零；加开关只会复制 webui auth 开关那类"关掉即裸奔/误解配"的分支。embed 形态（`core --webui`）同样武装——detached router 的状态订阅仍依赖它。

**D4 readiness 打点后移 + bind 失败复用 75 退出码。**
core 的 ready 行从"服务就绪"挪到"通道 bind 完成"之后；bind 失败以 `EXIT_BIND_FAILED`(75) 退出，supervisor 既有分类直接把 core 标 Degraded（与 webui bind 失败同语义），监督代码零改动。备选：ready 照旧、watchdog 周期探测 socket（深度健康检查）——更有普适性但引入探测协议与密钥分发问题，列为后续方向。

**D5 graceful exit 不删 secret 文件。**
socket 文件是"core 死了"的权威信号（优雅退出删 socket → 客户端报 `socket absent`）；残留 secret 文件无害——socket 不在时客户端根本走不到握手。少一个清理步骤就少一种残留状态。

**D6 全局横幅数据源：app-shell 自持 `/api/summary` 轮询。**
与 composer 的 reachability 轮询同间隔；不走 /ws 推送（reachability 入事件流是更大的接口改动，列为后续优化）。文案与交互对齐现有 ws-banner（全局、`role=alert`、恢复即消失）。

**D7 项目注册降级标记：201 响应体新增 `degraded: {cause}` 字段。**
老前端忽略新字段，无破坏；两者皆失败的 503 语义不变。

## Risks / Trade-offs

- [secret 文件可被同 uid 进程读取] → 与 env 实际强度等价（unix 同 uid 可读 `/proc/<pid>/environ`）；真实边界是 uid 校验 + 0600/用户 ACL，设计如实声明，不虚标强度。
- [双实例共享同一 config 目录] → socket bind 冲突先行暴露（75 → Degraded），secret 文件不会单独成灾。
- [Windows 无 0600] → secret 文件位于 config 目录，ACL 继承用户目录，与 env 的实际边界等价。
- [每次连接读文件的开销] → 本地小文件读取，微秒级；env 路径零变化。
- [新 webui + 旧 core（无自动武装）] → 行为同今天（socket absent），另有启动 warn，诚实性不变差。

## Migration Plan

1. core 自动武装 + secret 文件写入（含 config 解析）；
2. 客户端发现 + 重读（webui / im / router 订阅侧）；
3. readiness 时序 + bind 失败 75；
4. webui 横幅 + 降级标记；
5. 测试套件（进程 e2e 三旅程、浏览器 detached 旅程）与 AGENTS.md 简化。

回滚：还原二进制即可；env 注入路径全程保留，watchdog 既有部署行为不变。残留 secret 文件无害，可随手删。

## Open Questions

无。supervisor 深度探测（连通道验活）依赖本 change 的 secret 文件先落地，已列为后续方向，不在本 change 决策范围内。
