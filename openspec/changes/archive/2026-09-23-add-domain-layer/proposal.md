# Proposal: add-domain-layer

## Why

workspace 里没有任何 crate 能依赖根 crate `sebas`——根依赖了几乎全部成员，反向依赖即环。于是根里的共享类型与逻辑对 `sebas-webui` / `sebas-dispatch` / `sebas-router` / `sebas-node` 一律不可见，**复制是唯一出路**。实证：会话键编解码 `channel\0reference` 有 6 份实现、`expand_tilde` 有 2 份（注释直言「router 不能反向依赖 sebas root」）；`NodeView`/`NodeInfo`、`ModelAliasEntry`、providers.json 读取器各 2 份；project 有 4 种形状、session 约 10 种、provider 4 种、status 词表 8 种。全 workspace 仅 14 个 `impl From`，其中只有 1 个连接并行域类型；跨形状搬运靠 139 处 `serde_json::to_value` 与 253 处 `json!` 的约定维持。这既是漂移风险，也让根 crate 的 37.6k LOC 无法被任何后续拆分复用。

## What Changes

- 新增叶子 crate `sebas-domain`：project / session / provider / model 域概念的唯一定义处，附极少数中立原语（路径展开、时间戳）。
- **会话键编解码收敛为唯一实现**，落在既有叶子 crate `sebas-channels::key`（与 `ChannelKey` 同处），替换 6 份；`expand_tilde` 与 `now_unix` 各收敛为唯一实现。
- **中立契约类型从角色 crate 迁出、原位再导出**（`pub use`，调用点零改动）：dispatch 的 `SessionInfo` / `SessionEvent` / `TurnEntry` / `TurnStreamEvent` / `SessionIdentity` / `PendingApproval` / `PendingSubmission`，webui `session_backend` 的 `SessionRejection` / `PermissionNotice` / `PermissionDecision`，provider 状态词表中仍是词表的部分（`DefaultSelection` / `ProviderMode`）。
- **消灭显式重声明**：`NodeView` / `NodeInfo` 合一、`ModelAliasEntry` 合一、providers.json 读取器（router overlay）合一。
- **线格式与磁盘形状逐字节不变**：不改任何 JSON 字段名与标签、NDJSON 帧、SQLite 列名与亲和。`ProjectRow` 与 `ProjectEntry` 保持两个形状，改为同 crate 相邻 + 双向显式转换（合并留后续 change）。

## Capabilities

### New Capabilities

- `shared-domain-layer`: 中立共享域层的架构契约——每个域概念唯一定义、叶子依赖方向、可被主控角色与执行节点共同复用、线格式与磁盘形状不因重构改变。

### Modified Capabilities

（无。本 change 不改变任何对外可观测行为，故不触碰既有 spec。）

## Impact

- **新增**：`sebas-domain`（workspace 成员，叶子依赖：serde / serde_json / thiserror）。
- **改动**：根 `Cargo.toml` 与各 crate 清单加依赖；`sebas-channels::key` 增加编解码；`src/`（含 `node_link/projection.rs`）、`sebas-dispatch`、`sebas-webui`、`sebas-im`、`sebas-router`、`sebas-node-link` 的导入路径改为共享定义；删除 6 + 2 份重复实现。
- **不变**：所有线格式、磁盘形状、公开 HTTP/WS 契约、行为。验收 = 既有 `invoke testsuite-e2e` / `testsuite-acceptance` 全绿 + 快照零差异。
- **收益面**：解锁后续 change——`sebas-db`（持久层提取）、协议唯一之家（协议类型目前引用 dispatch/webui 类型，而 `dispatch → router` 已存在，不先搬出中立类型则共享协议 crate 必成环）。

## Non-goals

- **不拆根 crate 的角色实现**（`core_channel` / `node_link` / `watchdog` / `agent_backend` 的归属不动）。本 change 只把「共享层」抽出来，为后续拆分铺路。
- **不在本 change 内合并 `ProjectRow` 与 `ProjectEntry`**——合并会改磁盘形状与线格式，与本 change 的零变化基线冲突。该合并已交给 `migrate-project-registry`（项目记录）承担：本 change 只把两者搬到同一 crate 相邻位置、建立双向显式转换与形状钉测试，使后续合并成为机械操作。**`SessionInfo` 与 `SessionRow` 的合并不做**——`SessionRow` 带展示派生字段（`status_label` / `status_glyph` / `encoded_key` 等），合并会把展示关注点塞进共享域类型。
- **不做字符串词表类型化**（status / mode / element_type / approval 的 raw String 收敛）——单独 change。
- **不引入 protobuf 或任何编码变更**——见 `unify-ipc-protocol-home`。
- **不动 watchdog 控制 RPC 与 `src/ipc.rs`**——它们的类型已单一归属（客户端全在根 crate 内），无复制可消。
- **不改造 `sebas-node-link`**——它已是正确的共享契约叶子 crate，保持现状。
