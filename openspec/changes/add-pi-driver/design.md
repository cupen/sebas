# add-pi-driver — Design

## Context

sebas 的 agent 驱动层是 `AgentDriver` trait（`sebas-acp/src/agent_driver.rs`，唯一方法 `spawn`）+ 两个实现（`ClaudeDriver` 走 `cc-agent-sdk` 的 stream-json；`AcpDriver` 走 `agent-client-protocol` v1）。下游只消费统一的 `AcpEvent`/`AcpCommand` 词表，握手超时、审批路由、idle-kill 全在 driver 无关的 `SessionManager`。pi 官方无 ACP 支持（讨论 #4444 未立项、PR #836 曾被拒），通用 `acp` driver 接不进；但 pi 自带 headless RPC（`pi --mode rpc`，stdio 严格 JSONL，JSON-RPC 2.0 超集），命令/事件面覆盖 sebas 所需全部生命周期。研究结论与协议事实见 change 立项前的调研（对话已归档）：命令面 `prompt/steer/abort/new_session/get_state/get_messages/set_model/get_available_models/compact/…`，事件面 `message_update`（text/thinking/toolcall 增量）+ `tool_execution_*` + `agent_settled`。

## Goals / Non-Goals

**Goals:**

- 第三个一等 driver，产出与既有 driver 完全相同的词表，下游零分支。
- 产品默认 agent 切到 pi，且 claude 保留为随时可切回的一等选项。
- pi 特有缺口（无权限系统、自有凭据体系）如实呈现，不假装支持。

**Non-Goals**（承接 proposal）：

- 权限审批桥（pi 扩展 `tool_call` 拦截 + extension UI → 审批卡）另立 change。
- sebas-node 远端执行体支持、models.json 代写、安装配方自动化。

## Decisions

### D1：独立 `PiDriver`，不借道社区 ACP 适配器

备选：`driver = "acp"` + 社区 pi-acp 适配器（零代码）；等官方 ACP。否因：适配器是第三方 MVP、明言会有破坏性变更、被生产用户指出会话生命周期 bug，且把进程/回合语义外置给桥——做**默认 agent** 的地基不稳；官方 ACP 无排期。PiDriver 直说 pi 文档化的 RPC，协议形状可控可测。

### D2：协议映射表（驱动核心资产）

| pi RPC | sebas 侧 |
|---|---|
| `prompt` | `AcpCommand::send`（回合发起） |
| `steer` / `follow_up` / `clear_queue` | v1 不消费：排队语义留给 pi 自治 |
| `abort` | 取消路径；`agent_settled(aborted=true)` 后才发 `Finished` |
| `get_state` | 握手期学习 pi 会话 id（agent 侧会话 id 上报） |
| `set_model` / `get_available_models` | `SetModel` / 模型面（不内置硬编码模型表） |
| `message_update` 的 `text_delta` / `thinking_delta` | `TextDelta` / `ThinkingDelta` |
| `tool_execution_start/update/end` | `ToolStart` / `ToolProgress` / `ToolEnd` |
| `message_update` 的累计 usage | `UsageUpdate`（报多少是多少） |
| `compaction_*` / `auto_retry_*` / `summarization_retry_*` | 不发明新 `AcpEvent` 变体：v1 记日志 + 归入回合内呈现，`agent_settled` 仍是唯一回合边界 |
| 进程退出 / stdout EOF | 带 terminal 标记的 `Error` |
| `get_commands` | `AvailableCommands`（会话命令面板，诚实退化） |

### D3：会话身份与恢复

sessions 目录经 agent 配置钉住（`--session-dir`），pi 会话文件天然持久。握手上报 `(routing id, resumed, acp_session_id = pi 会话 id)`，对齐 `AcpDriver` 的 session/load 语义；恢复 = spawn `--session <id>`，被拒（文件缺失/损坏）诚实回落新会话 + resumed=false。不用 RPC `switch_session` 做恢复（spawn 期挂接更简单，进程边界即会话边界）。

### D4：v1 无权限，如实呈现

pi 无权限系统：不产生 `PermissionRequest`，`SetMode` 应答非终态「不支持」（复用 `AcpDriver` 的口径与错误形状），看门狗每秒 `set_permission_mode` 探针因此天然被容忍。UI 对 pi 会话如实呈现「无权限系统」。

### D5：凭据零翻译 + reachability

不读写 pi 的 `models.json`/auth 存储（一个文件一个写入者）；子进程继承 sebas env（pi 自读标准 API key env 表）。reachability：二进制在场（复用既有探测）+ `pi auth check`（ready/not_ready/invalid）区分「装了没登录」。备选「翻译 ProviderResolution → env」：留待有真实需求再立项（claude 有先例，机械）。

### D6：默认 agent 解析链

显式 `[acp] default` > 单 agent 隐式默认 > 多 agent 无 default（取优先链 `pi`→`claude` 中已配置者，都未配则取已配置 agent 字典序最小者）> 零 agent 回退探测链 `pi` → `claude`（二进制可解析为准，命中写启动日志，皆缺按既有口径失败）。示例配置种子 `pi`（默认）+ `claude`（保留）。启动期对默认 agent 二进制的硬失败检查保留，但缺省/隐式默认不可达时在目录与 spawn 处如实报因，不静默改选。备选「直接翻转常量为 pi」：会让零 agent + 无 pi 的存量环境拒绝启动，探测链避免该回归——多 agent 无 default 的存量 config（如 e2e 沙箱的 claude+fakeacp）同理必须落到已配置的 agent，否则会去探测未配置的 pi 而启动失败。

### D7：模块落点

`sebas-acp/src/pi/`（协议编解码 + driver，对齐 `claude/`、`acp_driver/` 的组织）；config `AgentConfig::Pi(AcpPiConfig)`（path/sessions_dir/work_dir/args 键值形式/两超时，镜像 claude 形态）；装配点两处 match（`run.rs::build_agent_registry`、`agent_store.rs::ensure_registered`）；`sebas-models` `is_valid_driver` + `AgentDefinition::command`；`sebas-dispatch` 写入口校验放行；WebUI `settings-modal.ts` 表单加 pi 形态。`/api/agents` 契约不变（driver 不上 wire，既有规范）。

### D8：帧解析纪律（pi 文档明示的陷阱）

严格按 LF 分帧、容忍前导 CR；**不得用会把 U+2028/U+2029 当行边界的通用行读取器**（Node readline 语义）；stdout 持续读（停读会拖死 pi 的背压）；stderr 只作诊断不解析。命令关联用 `pi.id`（异步乱序），事件一般无 id。

### D9：测试策略

- 协议编解码/翻译层：录制帧 fixture 离线单测（对齐 ipc golden fixture 仓库惯例），锁事件→`AcpEvent` 映射形状。
- driver 集成：spawn 真二进制的用例 **skip-if-absent**（pi 不在场跳过并在输出如实说明，不假装通过）。
- 沙箱验收：pi 侧配自定义 provider（其 `models.json` 指向 `sebas fake-provider`）实现零真实凭据全链路；`PI_CODING_AGENT_SESSION_DIR` / `--session-dir` 与 HOME 钉进沙箱。

### D10：版本基线与漂移

协议基线 = 仓库 main 分支文档（stdio JSONL）。pi.dev 在线文档另述 AF_UNIX socket 拨号形态——版本差注记，实现以装好的 `pi --help` 为准；fixture 单测即漂移闸门（pi 升级改形状时必红）。运行前提 Node ≥22.19 如实探测报告。

## Risks / Trade-offs

- [pi RPC 是私有协议、无兼容承诺] → fixture 锁形状 + 版本记录在 reachability 元数据；升级漂移显式红灯而非静默错译。
- [默认 agent 全权限直跑的安全面] → proposal/spec 明示 v1 无审批；权限桥已列为后续 change 的第一优先；文档与 UI 如实标注。
- [零 agent 回退探测链引入 PATH 依赖的解析时机] → 探测只在启动配置解析期做一次、结果写日志，运行期不再变。
- [pi 会话文件由 pi 自管，sebas 不知情其损坏] → 恢复被拒即诚实回落新会话（D3），不重试不伪装。
- [默认切换 BREAKING：抄新示例的存量部署] → 显式 `default` 永远赢；claude 条目保留；回滚 = 还原回退常量与示例种子。

## Migration Plan

1. 合入即生效：新示例种子 pi 默认；存量配置（有显式 default 或已有条目）行为不变。
2. 升级前检查项写入发布说明：确认 pi 安装（`npm i -g @earendil-works/pi-coding-agent`，Node ≥22.19）或显式声明 `[acp] default`。
3. 回滚：revert 本 change 即恢复 claude 回退；pi agent 行（settings.db 种子）无害残留，可经 Settings 删除。

## Open Questions

- pi 安装配方挂靠 add-agent-auto-install 的加行时机（该 change 落地后小 PR 即可，不影响本设计）。
- AF_UNIX socket 拨号形态是否值得在驱动内支持（等 pi 正式发布该形态且有真实需求再评估）。
