## Context

前三批已建立 11 个 capability（含网关数据面/控制面）。本批 4 个覆盖控制平面与运维面，批内关系：

```
cli-service ──入口──▶ watchdog ──监管──▶ core（run）
     │                    │ spawn + control RPC
     │                    ▼
     └──replay-debug   webui（admin 动作走 control RPC 回 watchdog）
```

## Goals / Non-Goals

**Goals:**

- watchdog 以「控制面契约」为纲：子进程监管、升级/回滚安全、RPC 鉴权、确认授权、事件时间线、bare-core 降级——全部只写已实现行为
- webui 写清两件事：本地安全基线（loopback-only + 可选 admin 密码）与「standalone 模式会话操作是离线快照」这一事实语义
- cli-service 以子命令树 + systemd 安装 + 配置发现为契约；`--user` 降权是安全语义
- replay-debug 以「录制格式 + 回放保真 + 副作用边界」为契约

**Non-Goals:**

- 不写未接线机制的设计意图（ServiceSet/ServiceRestart 的 Phase 4 蓝图、WebUiLifecycle 守卫、签名断言机制——spec 只记录当前拒绝/缺席行为）
- 不写死配置字段（`max_retries`/`retry_delay_secs`/`check_on_start` 在代码中无消费者，不进 spec）
- `sebas record`（ACP stdio 录制，测试夹具生成工具）不属于本批 capability——它是开发期工具，不是运行面行为

## Decisions

### D1: upgrade-command 并入 watchdog

命令解析与转发语义已在 `router-commands` 覆盖；升级/回滚的执行、超时、自动回滚、进程模型是 watchdog 控制面的一半，单独成 spec 必然两边重复。watchdog spec 内以「升级执行」requirement 承接。

### D2: 只记录已实现行为，未实现的面写成「当前拒绝」

研究确认多处「有接口无实现」：ServiceSet/ServiceRestart 恒返回 `service_unavailable`；RPC 幂等 helper 存在但未接入 wire 路径；签名断言机制完整但无人调用。spec 对这些写当前可观察行为（拒绝/缺席），不写蓝图。这是「只反映当前状态」原则在控制面的应用。

### D3: webui 的两态语义分开写

`run --webui`（共享活 router）与 watchdog spawn 的 standalone webui（自建 RouterHandle、丢弃出站通道）是两种不同行为。spec 以「standalone 只读快照 + 本地映射操作」如实记录，不美化成完整功能。

### D4: 裸 `sebas` 不运行核心

clap 无默认子命令——裸 `sebas` 是解析错误。CLI spec 明确记录此行为（与直觉的「无参数跑服务」相反）。

## Risks / Trade-offs

- [watchdog 面大（RPC/升级/回滚/监管/服务）] → 按域拆 requirement，每条独立可审
- [研究 agent 标注多处 doc-vs-code 偏差（socket 回退路径、幂等、断言）] → 一律以代码为准，偏差进最终报告
- [webui 安全面（CSRF token 未铸入模板、全局限速器）属实现瑕疵] → spec 记录实际生效的防线（loopback origin 路径），瑕疵另行报告

## Migration Plan

纯文档。validate --strict → archive。回滚 = 删对应 4 个目录。

## Open Questions

无。
