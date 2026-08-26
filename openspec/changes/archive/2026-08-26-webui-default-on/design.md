# Design: webui-default-on

## Context

WebUI 生命周期目前由三处 opt-in 入口控制：`run --webui` flag
（`src/cli.rs`，默认 false）、watchdog 配置
`[watchdog.webui] enabled`（`src/config.rs:213`，serde default false）、
以及独立 `sebas webui` 子命令。watchdog 通过 `WebUiSpawner`
（`src/watchdog.rs:136`）spawn 子进程并注入 control secret。

Proposal 详见 proposal.md。

## Goals / Non-Goals

**Goals**

- watchdog 形态下 WebUI 零配置可用，端口冲突不影响核心服务。
- 保留显式关闭路径（配置 `enabled = false`；CLI `--no-webui`）。

**Non-Goals**

- 不改 WebUI 安全基线（loopback、admin 密码、POST-only）。
- 不改裸 `sebas run` 默认行为。

## Decisions

### D1: serde default 翻转，而非 `Option<bool>` 三态

`WatchdogWebUiConfig::enabled` 的 serde default 改为 `true`，同时
`Default::default()` 同步翻转，保证「无配置段 = 启用」与「显式 false =
关闭」两态即可。备选是 `Option<bool>` + `unwrap_or(true)`，可区分
「未设置」与「显式 false」，但当前没有任何行为需要区分这两者，引入
三态只会增加配置解析与测试的组合空间。

### D2: 子进程退出码分类 → degraded，而非 spawn 失败分类

当前 design 描述的「spawn 失败后分类为 degraded」与实际控制流不符。

实际流程：
```
spawn_aux_process()         → 只启动子进程，bind 在子进程内部
  child.wait()  → Exit::Crashed(Some(code))  → 走 crash backoff 无限重试
```

修正后的设计：

1. WebUI 子进程在 bind 失败时以特定退出码退出（如 `EXIT_BIND_FAILED = 75`，
   与其它 crash 区分）。
2. Supervisor 的 `Exit::Crashed(code)` 分支检查该退出码：若匹配则标记
   `ServiceState::Degraded`，停止 spawn 重试，记 warning 日志。
3. 控制平面 restart 请求复位 degraded 状态，重新 spawn（重试 bind）。
4. 非 bind 类退出码仍走原 crash backoff 路径。

备选方案：watchdog 预探测端口可用性再 spawn（TOCTOU 竞态，且多一次
探测逻辑）；stderr 文本匹配（脆弱，依赖 i18n）。退出码分类最可靠。

### D3: `--no-webui` 是 CLI 对称性糖

裸 `run` 默认不启动 WebUI，`--no-webui` 仅作为 flag 存在被解析
并与 `--webui` 互斥（同时给出时报错），值为 true 时等于旧行为。避免
用户从 watchdog 文档迁移到裸 run 时产生「为什么这里关不掉」的困惑。

### D4: degraded 状态经控制平面 service status 暴露

复用既有 `service_status` / control RPC 的服务状态模型。`ServiceState`
新增 `Degraded` 变体；control RPC 的序列化/反序列化同步更新。

### D5: ownership guard 就是端口绑定本身

spec 要求「防止 watchdog 与遗留 `run --webui` 双启动」。当前实现中
两者共用同一端口（`127.0.0.1:9797`），端口绑定天然就是互斥锁：

- 若 watchdog 的 WebUI 子进程先占住端口，`run --webui` 的 bind 失败
  → 用户看到明确错误信息。
- 若 `run --webui` 先占住，watchdog 的 WebUI 子进程 bind 失败 →
  退出码 75 → supervisor 标记 degraded。

不需要额外锁文件或 IPC 注册。遗留 `run --webui` 的 bind 失败返回
`SebasError` 让用户看到「端口已占用」即可。

### D6: 合并 `should_start_watchdog_webui` 与 `WebUiEndpoint::from_config`

两者都检查 `config.enabled`，是重复逻辑。翻转默认值后，将
`should_start_watchdog_webui` 内联到 `run_watchdog` 中直接读
`config.webui.enabled`，`WebUiEndpoint::from_config` 改为
`WebUiEndpoint::from_config_unchecked` 或直接构造 endpoint。

## Risks / Trade-offs

- [9797 端口被本机其他程序占用] → D2 的退出码分类降级；发布说明提示
  可改 `port` 或 `enabled = false`。
- [既有用户升级后多出一个常驻进程] → 行为级 BREAKING 在发布说明标注；
  配置显式 false 即回退。
- [WebUI 自身 bug crash 与 bind 失败混淆] → 退出码 75 仅用于 bind 失
  败，其余退出码仍走原 crash 路径，不会静默 degraded。
- [双进程同时 bind 的竞态] → 内核保证了 bind 的原子性，胜者继续、
  败者退出码 75。

## Migration Plan

1. 翻转默认值 + 退出码降级 + `--no-webui` + ownership guard 文档，一次发布。
2. `config/config.toml.example` 注明新默认。
3. 回滚：用户侧 `enabled = false`；代码侧 revert 单个 commit。

## Open Questions

（无）