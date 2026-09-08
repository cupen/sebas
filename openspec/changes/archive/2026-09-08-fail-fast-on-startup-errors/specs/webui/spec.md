## ADDED Requirements

### Requirement: web_spawn 失败的立即内显
webui 在为会话派生 acp 子进程（web_spawn）时 SHALL 把 spawn failure 立即 inline 到 transcript 作为可读错误事件，**不**延后到后续 Removed 事件或会话状态变更。错误事件 SHALL 包含失败原因（exit code / stderr 末行 / spawn error message）与时间戳；transcript SHALL 在新子进程的未成功期间持续呈现该错误而不被后续成功 turn 隐式覆盖。Removed 事件仍可作为次要信号补充，但 SHALL NOT 作为 spawn failure 的首次呈现路径。

#### Scenario: spawn failure surfaces immediately in transcript

- **WHEN** webui 为某会话派生 acp 子进程失败（exit code 非零或 spawn 系统调用失败）
- **THEN** transcript SHALL 立即出现一条错误事件，含失败原因；会话状态 SHALL 标记为 spawn-failed（而非仍按 placeholder/empty 假装存在）

#### Scenario: repeated failure does not silently retry

- **WHEN** 同一会话连续 web_spawn 失败 2 次以上
- **THEN** transcript SHALL 累计呈现失败计数与最近一次原因，**不**静默重试；操作员 SHALL 在 UI 上明确看到 spawn 已失败

#### Scenario: spawn success after prior failure clears error

- **WHEN** 一次失败的 spawn 之后用户重试并成功派生
- **THEN** 新 turn 正常进入 transcript；之前的 spawn-failed 错误事件保留为历史（不删除），但会话状态恢复为非 spawn-failed

## MODIFIED Requirements

### Requirement: Session backend seam
The webui SHALL keep session state in the backend (the core is the source of truth for session lifecycle) and the frontend SHALL render server-driven events faithfully. **补充**：backend 推送的 spawn-failure 事件 SHALL 在前端被理解为「启动失败」立即显式呈现——前端 SHALL NOT 把 spawn failure 静默合并到后续 placeholder 状态；前端 SHALL NOT 在 spawn failure 与 Removed 事件之间存在时窗内显示「会话正常创建」的瞬态。

#### Scenario: backend push renders faithfully

- **WHEN** backend 推送 spawn-failure 事件
- **THEN** 前端立即渲染该事件为 transcript 内显错误，不等待 Removed 事件；会话状态 SHALL 在 backend 推送 Removed 之前已经标记为 spawn-failed

#### Scenario: WebUI is testable without a core

- **WHEN** the WebUI's route tests run
- **THEN** they drive routes through a fake backend, with no ACP child, no socket,
  and no state file

#### Scenario: no backend leaks into templates

- **WHEN** a page is rendered
- **THEN** which backend is in use is not visible in the markup except where the
  channel's degradation contract requires stating that the core is not connected