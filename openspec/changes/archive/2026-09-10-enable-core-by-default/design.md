## Context

core 默认关的历史理由随 `extract-im-service` 失效：飞书适配器已迁去 im 服务，core 是纯会话核心，无飞书凭据也能正常启动（`[feishu] enabled = false` 的沙箱配置已在 e2e 中验证）。既然 core 应永远在场，`[watchdog.core] enabled` 这个「能关掉会话核心」的入口本身就是多余的——它只会制造 webui 半残（session 不可达）的合法但无意义形态。watchdog 的 ServiceManager 三层期望态合成与 fail-fast 链路（spawn 连败 → `failed-startup` 终态 → 全部停机 + exit 75）均已存在，本次删除 core 的启停开关并把 fail-fast 固化为 core 的显式 spec 要求。

## Goals / Non-Goals

**Goals:**
- core 恒由 watchdog 拉起并监督，无 config / 服务页 / ctl 的停用入口；core 启动失败立刻告警并 fail-fast。
- 配置面简化：`[watchdog.core]` 只剩 `channel_path` / `secret_file`。

**Non-Goals:**
- 不动 router / im 的默认判定与开关，不动 core 的 `RestartCore` 确认路径与 auto-rollback，不新增告警渠道（见 proposal Non-goals）。

## Decisions

- **D1：删除 `enabled` 键而非仅翻默认值。** 只翻默认值（default→true）仍保留「显式 false 关 core」的合法形态，那正是要消灭的半残形态。`WatchdogCoreConfig.enabled` 字段删除，core spec 恒 `DesiredState::Enabled`，不读 config。备选「保留键、仅默认开」被否：入口还在，半残形态就还在。
- **D2：services.json 的历史 `core: off` 覆盖一并失效。** 三层合成对 core 不再合成——core entry 注册时忽略 config 与 persist 层，恒 enabled。webui 前端对 core 行不渲染 enable/disable；`ServiceSet`/`ServiceRestart` 命名 core 仍按既有规则拒绝（指向 `RestartCore`）。备选「尊重历史 off」被否：与 D1 矛盾，等于换个地方保留开关。
- **D3：失败告警复用既有 fail-fast，不发明新链路。** spec 层面把「core spawn 连败 → failed-startup → exit 75 + ctl status 摘要」命名为 core 的专属 requirement；代码零改动，仅 tasks 核对 core 场景单测覆盖。备选「core 首次失败即退出」被否：丢失 crash-loop 退避语义。
- **D4：配置兼容按「失效并告警」处理。** 升级后既有 `[watchdog.core] enabled = false` 成为未知键——serde 忽略（deny_unknown_fields 未开）+ 启动时打一条 deprecation warn；`services.json` 的 `core: off` 读取时被过滤并 warn。不做硬报错：避免既有部署升级即崩。
- **D5：e2e/沙箱注入逻辑删除。** `tests/support/mod.rs::enable_supervised_core`、`tasks.py` 沙箱配置里的 `[watchdog.core] enabled = true` 注入全部删除（core 默认即起）；依赖「无 core 起 webui」的用例改写为恒有 core。

## Risks / Trade-offs

- [core 启动失败必拖垮整个 watchdog（含 webui），不再有「关 core 保 webui」选项] → 有意选择：core 恒在，半残 webui 无意义；缓解是 max_spawn_failures 可调 + ctl status 可诊断 + auto-rollback 兜底升级场景。
- [sandbox/e2e 大量依赖 core 缺席，重写面大] → 一次性成本；D5 删除注入逻辑后沙箱反而更简单（少一个 patch 步骤）。
- [既有部署 `enabled = false` 升级后 core 突然拉起，占用资源] → D4 的 deprecation warn 提前告知；core 无凭据需求、bare-core degraded mode 兜底，拉起是安全的。

## Migration Plan

软迁移：旧 `enabled` 键与 services.json `core` 项被忽略并打 warn，不报错；一个版本周期后可考虑清理。回滚 = 还原代码 + 重新接受 `enabled` 键。
