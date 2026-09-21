# close-acceptance-blind-spots Proposal

## Why

2026-09-21 真实部署事故暴露验收体系的结构性盲区：三套测试全部跑沙箱（假 provider、哑
key），「真实部署可用」没有主人——shell 继承的 `ANTHROPIC_*` env 把 Claude 子进程引向
公司网关、模型名未路由，最基本对话失败而五簇验收 100% 全绿。另有三个次级缺口：继承 env
生效完全不可观测；用户动作反馈无时限约束（`/code-review` 零回音、错误 3 分钟才落）；
持久化 spawning 状态跨重启无人收敛（真机实例存在僵尸 spawning 会话）。

## What Changes

- fake-claude 桩扩展为全行为 mock：场景参数化模拟正文流式、thinking、tool_use/tool_result
  工具环、空响应、慢响应、错误响应，全部测试层零真模型调用
- core/webui 启动时检测「会引导子进程的继承 `ANTHROPIC_*` env 且当前模式不覆盖」→ 显式 WARN
- 回合以零可见输出收尾时，会话投影追加合成提示条目；任一提交面 5 秒内必有可见反馈
- 恢复路径对持久化 spawning 状态强制落定（重投 spawn 或合成失败条目），不得永久悬挂
- `invoke smoke-real` 真实环境冒烟配方：operator 手跑、不进 CI、agent 不碰真实凭据
- 核心功能命中硬指标从 ≥90% 提到 100%（豁免仍不计分母），账本按当前 spec 基数重算
- 新增仓库 QA 验收技能 + subagent：启动先询问验收范围（核心五簇/非核心/全量/快速冒烟）

## Capabilities

### Modified Capabilities

- `cli-service`：启动时 env posture 告警
- `session-lifecycle`：重启 spawning 收敛；零输出回合合成落点
- `live-turn-stream`：提交反馈时限
- `testsuite-process-e2e`：fake-claude 全行为场景契约
- `testsuite-acceptance`：100% 硬指标；真实环境冒烟入口；QA 验收技能与分派入口

## Non-goals

- 不改沙箱规矩：自动化测试仍零真实凭据；冒烟配方由 operator 手跑
- 不写平行的「QA 验收规范」文档（与覆盖账本、三套测试 spec 重复）
- 不动 operator 运行实例；存量僵尸 spawning 会话随新恢复语义在下次重启自然落定
- 不实现 capability-tier 模型注解（claude-env-cover 已留待后续）
