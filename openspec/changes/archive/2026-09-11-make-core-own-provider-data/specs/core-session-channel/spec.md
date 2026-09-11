## MODIFIED Requirements

### Requirement: State store channel surface
core session channel SHALL 暴露 state-store 引擎的 snapshot / mutation / subscribe 三类 RPC：`StateSnapshot { domain }` 返回该 domain 的当前快照，`StateMutation { domain, payload }` 应用变更，`StateSubscribe` 启动变更推送流；服务端 SHALL 把请求路由到 state-store engine 实例并以 `StateSnapshot / StateMutationOk` 帧回包。订阅流 SHALL 在首帧之后推送该 domain 的后续 mutation 帧，且 SHALL 在 mutation 失败时回 `Rejected { kind: ... }` typed rejection（不静默吞错）。三类 RPC SHALL 走与 session RPC 同样的 secret+peer-uid 鉴权。provider / model / alias 数据 SHALL 经 `providers` 与 `aliases` 域管理，默认 provider 与默认 model（defaults）SHALL 并入 `settings` 域；这些域 SHALL 是 provider 数据唯一的写入通道，core 以外的进程 SHALL NOT 经文件或其它路径直接改写该数据。

#### Scenario: StateSnapshot returns current snapshot

- **WHEN** 客户端发 `StateSnapshot { domain: "agent-defaults" }`
- **THEN** 服务端以 `StateSnapshot { domain, payload }` 回包；payload 与 state-store engine 持有一致

#### Scenario: StateMutation applies change

- **WHEN** 客户端发 `StateMutation { domain: "agent-defaults", payload: {...} }`
- **THEN** 服务端应用变更、回 `StateMutationOk`；下一次 `StateSnapshot` 看到新值

#### Scenario: StateMutation rejected does not silently swallow

- **WHEN** `StateMutation` 携带非法 payload（如未知字段 / 类型错误）
- **THEN** 服务端回 `Rejected { kind: ... }` 含 typed rejection；engine 状态未变

#### Scenario: StateSubscribe delivers mutations after snapshot

- **WHEN** 客户端先发 `StateSubscribe { domain }`、收到首帧 snapshot；服务端在订阅期间应用 mutation
- **THEN** 客户端收到 mutation 帧（带 domain + payload）；滞后 SHALL 走 lag-disconnect 路径

#### Scenario: provider management runs over the channel

- **WHEN** WebUI 或飞书 `/provider` 卡片新建、改名、删除一个 provider，或设置默认 provider / 默认 model
- **THEN** 变更经 `StateMutation` 落在 `providers` / `aliases` / `settings` 域，随后的 `StateSnapshot` 读到新值，且 router 经订阅在无重启下生效

#### Scenario: defaults round-trip with provider data

- **WHEN** 客户端经 `settings` 域写入默认 provider 与默认 model
- **THEN** 该默认值与 provider 数据同库持久化，core 重启后仍可读，且不产生独立的 defaults 文件
