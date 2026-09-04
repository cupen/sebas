# add-opencode-acp

## Why

现有 `AcpDriver` 已能驱动任意原生 ACP agent（`agent-client-protocol` v1），但只验证过 Gemini/Codex 类实现；opencode（sst/opencode，`opencode acp` 子命令）是标准 ACP v1 agent，接入成本极低。真正的缺口是 **ACP resume 未实现**——`AcpDriver::spawn` 对 `resume=true` 直接 `warn` + fresh start，而 opencode 的 ACP `loadSession` 能力完整。接 opencode 顺带把 ACP 通用 resume 补上，一举兑现「配置即可新增三方 agent」的承诺。

## What Changes

- **接入 opencode 作为 ACP agent**：配置 `[acp.agents.opencode] driver="acp", command=["opencode","acp"]` 即可；`agent-kinds list` / webui 创建会话下拉天然识别（`opencode --version` 输出裸版本号，与现有 `discover_agent` 探测兼容）。
- **实现 ACP 通用 resume**（`acp-driver`）：`SessionStart::Load` 时经 ACP `session/load` 恢复既有会话，取代当前「warn + 全新会话」；load 失败（会话不存在/agent 不支持）沿用现有回退语义（fresh + `resumed=false`）。opencode 的 `loadSession`/`resumeSession` 能力完整，是本能力的首个受益者。
- **验收流程设计**：为本次接入与 ACP resume 设计可重复的验收流程——自动化（mock ACP agent 回归 + 现有测试套件扩展）与真实 opencode 冒烟两档，写入 tasks 与 design，确保「配置即可用」的质量门槛。

## Capabilities

### New Capabilities

- `opencode-agent`: opencode 作为受支持 agent 的接入契约——配置形态、版本探测语义、与 ACP 驱动的组合验证。

### Modified Capabilities

- `acp-driver`: 「Resume」从未实现（诚实 fresh）升级为经 `session/load` 恢复会话的完整能力；load 失败仍回退 fresh。

## Non-goals

- 不做 ACP resume 之外的协议扩展（fork/close/list、usage、plan 等保持现状）
- 不为 opencode 注入私有配置（如 `--model`、provider 透传由现有 `extra_env` 机制覆盖，不新增）
- 不改飞书侧路由与卡片交互
- 不做 opencode 专属故障诊断/日志增强（沿用通用驱动错误语义）
- 不做数据迁移或历史会话导入

## Impact

- `sebas-acp/src/acp_driver/`: spawn 路径增加 `session/load` 分支；`codec` 可能需要处理 load 相关响应
- `sebas-acp/src/claude/manager.rs`: resume 回退语义不变，但 ACP 路径的 `resumed` 判定从恒 false 变为真实值
- `src/run.rs` / 配置：无代码变更（配置示例/文档新增 opencode 条目）
- `sebas-webui/src/agent_kinds.rs`: 现有 `--version` 探测兼容 opencode，无需改动（以测试确认）
- 依赖：无新增（`agent-client-protocol` 已含 load 能力）
- 测试：mock ACP agent 夹具扩展 + opencode 真实冒烟脚本（可选）
