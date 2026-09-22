## Context

动机见 `proposal.md` — Why。设计只需要以下当前状态与硬约束：

1. **根 crate 是依赖图的死端。** 根 `sebas` 依赖全部 12 个成员 crate，因此没有任何成员能依赖根；`src/` 下 37.6k LOC 对 `sebas-webui` / `sebas-dispatch` / `sebas-router` / `sebas-node` 不可达。这是全部复制的成因，也是本设计的出发点。
2. **执行节点的依赖纪律是已生效的 spec 要求**（`openspec/specs/execution-node`「Execution node process persona」）：节点产物内**不得出现主控角色（core / webui / router / im）的实现**，机械核对方式是 `cargo tree -p sebas-node`。
3. **协议类型今天咬住角色 crate。** `src/core_channel/protocol.rs:29,33` 直接 `use sebas_dispatch::{SessionInfo, TurnEntry, SessionEvent, SessionIdentity, PendingApproval, PendingSubmission, TurnStreamEvent}` 与 `use sebas_webui::session_backend::{PermissionDecision, PermissionNotice, SessionRejection}`。而 `sebas-dispatch → sebas-router` 这条边**已经存在**。
4. **既有中立叶子 crate 的先例**：`sebas-channels`（`ChannelKey` / `ChannelEvent` / `ChannelCard`）、`sebas-node-link`（两侧共用的链路契约）、`sebas-startup`（节点可安全依赖的叶子）。它们的注释都明说「抽出正是为了让另一方不必复制」。
5. 会话键编码的 6 份实现**并非全部等价**（见 Risks）。

## Goals / Non-Goals

**Goals:**

- 让「每个共享域概念只有一处定义」成为**可机械核对**的事实，而不是约定。
- 把中立契约类型迁到一个**每个人都能依赖的叶子**，从而为 `unify-ipc-protocol-home` 解开环（见 Decision D4）。
- 在**零对外可观测变化**的前提下完成搬迁：线格式与磁盘形状逐字节不变。

**Non-Goals:**

- 不追求类型数量减少（把 4 个 project 形状变 1 个）——那是行为与存储变更，见 `proposal.md` — Non-goals。
- 不做 `sebas-node-link` 的归并或重命名。
- 不引入编译期依赖检查的新机制（`cargo tree` 断言即可，见 tasks 2.6）。

## Decisions

### D1 共享层是**新叶子 crate** `sebas-domain`，不是根内模块，也不叫 `sebas-core`

- **为何不是根内模块**：由 Context 1，根内模块对其他 crate 依然不可达，等于只整理不共享——6 份会话键、2 份 `expand_tilde` 一份都不会消失。用户原话「增加 sebas-core 模块」，但「模块」在此行不通，必须是 crate。
- **为何不叫 `sebas-core`**：Context 2 是已生效的 spec 要求，节点产物内不得出现 `core` / `webui` / `router` / `im` 角色的实现。一个名为 `sebas-core` 的 crate 一旦被节点依赖，`cargo tree -p sebas-node` 的机械核对就失去了意义——检查项与依赖名撞车，日后没人分得清那行输出是「协议类型」还是「主控实现」。且 `sebas-router` / `sebas-webui` 的命名惯例是**角色名 = 角色实现**，`sebas-core` 会被读成「core 角色的实现」。
- **被否备选**：拆成 `sebas-identity` / `sebas-provider` / `sebas-session-model` 多个小 crate。边界更窄、依赖更精，但 crate 数从 15 涨到 18+，每个都要接进 `cargo tree` 纪律与 CI，收益不抵编排成本。**触发条件**：若 `sebas-domain` 的模块间出现真实依赖方向冲突（例如 identity 需要 provider 而 provider 不需要 identity 却被迫同包），再拆。

### D2 会话键编解码落 `sebas-channels::key`，不落 `sebas-domain`

它编码的是 `channel\0reference`——**通道身份**，语义上正是 `sebas-channels` 的职责（`ChannelKey` 已在该文件）。该 crate 已是叶子，且 6 个消费方里已有 4 个（webui / dispatch / im / feishu）依赖它；root 与 node 加这条依赖也合法。**被否备选**：放 `sebas-domain`——会让 domain 变成「身份 + 域对象」的混合体，而 channels 是更早、更专的归属点。

### D3 迁移战术：类型搬到新 crate，原 crate `pub use` 原位再导出

搬迁不触碰调用点：`sebas-dispatch::SessionInfo` 在源码里仍可用，只是定义改为 `pub use sebas_domain::SessionInfo`。这把一次全 workspace 的大爆炸拆成**逐 crate 可编译、可测试的小步**，每步都能跑该 crate 的测试与线格式快照。**被否备选**：一次性全量替换 import 路径——改动面相同但中间状态不可编译，失去分步验证能力。

### D4 必须先把中立契约类型搬出角色 crate——这是被验证出来的环，不是洁癖

`src/core_channel/protocol.rs` 引用 dispatch 与 webui 的类型（Context 3）。任何承载这些协议类型的共享 crate 因此必须同时依赖 dispatch 与 webui；而 `sebas-dispatch → sebas-router` 已存在，webui 也依赖 router，于是：

```
router ──▶ protocol ──▶ webui ──▶ router        ← 环
router ──▶ protocol ──▶ dispatch ──▶ router      ← 环
```

**结论**：`SessionInfo` / `TurnEntry` / `SessionEvent` / `SessionIdentity` / `PendingApproval` / `PendingSubmission` / `TurnStreamEvent` / `SessionRejection` / `PermissionNotice` / `PermissionDecision` 必须先落到一个**所有角色都能依赖的叶子**，协议 crate 才可能共享。这正是本 change 必须先于 `unify-ipc-protocol-home` 的原因，也是它被单独立项的理由。

### D5 持久层边界：domain 放「形状 + 中立原语」，`sebas-db`/`sebas-models` 放「runtime + ActiveRecord」

- 进 domain：域对象的形状、provider 状态词表中**仍是词表的部分**（`DefaultSelection` / `ProviderMode`）、providers.json overlay 的**读取器**。
- **不再进 domain：`PersistedState` / `Item`**——本计划后续已决定由 ActiveRecord 取代无类型 JSON 载体（`extract-sebas-db` D3b：读写一律经 `ProviderRow`；`single-state-dir` 把 providers 扁平化为类型化列）。搬一个注定要删的形状是白搬，故本 change 对 provider 词表只搬上述两项。
- 不进 domain：`src/sebas_state/{db,migration,writer,repo}.rs` 的 SQLite 连接配方、schema 注册表、迁移 diff、单写线程 actor——全部留给 `extract-sebas-db`；各表的 ActiveRecord struct 归 `sebas-models`（同 change D2 的分界）。
- **overlay 读取器放 domain 的理由**：router 必须够得着它（今日靠复制 `ProviderOverlay`），而它是角色中立的；放 `sebas-db` 也可行但会与 change 3 的时序纠缠——change 1 的验收要求 router 的复制消失，不该等 change 3。**已知代价**：JSON 文件持久化的归属因此横跨两个 crate，`extract-sebas-db` 可再迁（记录在 change 3 的 Context，不在此强求）。

### D6 本 change 不合并 `ProjectRow`/`ProjectEntry`（合并交给 `migrate-project-registry`），也不合并 `SessionInfo`/`SessionRow`

本 change 的零变化基线（spec「Wire and on-disk compatibility preserved」）不允许改磁盘形状或线格式，而合并这两个形状必然要改其一。因此本 change 只做**同 crate 相邻 + 双向显式转换 + 两侧形状各钉一个测试**——这恰好也是让后续合并成为机械操作的前置。

**推迟的触发条件已变**：原文写的是「需要统一存储形状时，必须先有 schema 版本 bump 与迁移路径——即等 `harden-schema-migration` 落地之后」。**产品尚未发布**，该门槛消失：改 schema 的代价只剩开发机上一次重置。因此项目记录的合并**已指派给 `migrate-project-registry`**（它本来就要重建 `projects` 表以加入节点维度），本 change 不再持有它。

`SessionInfo` / `SessionRow` 的合并不做：`SessionRow` 承载展示派生（`status_label` / `status_glyph` / `encoded_key`），合并会把展示关注点塞进共享域类型。正确形态是「规范类型 + 一处转换」，即本 change 已在要求里写下的规则。

### D7 `expand_tilde` / `now_unix` 落 `sebas-domain::prim`，承认这是轻微异味

两处复制（根 `src/config.rs:1112`、`sebas-router/src/config.rs:968`、`sebas-dispatch/src/state_store.rs:310`）的成因是「够不着根 crate」，而它们确实是角色中立的原语。放 domain 会让「域层」里出现路径工具——**被否备选**：独立 `sebas-util` crate（更纯），因 crate 数与 CI 编排成本否决。**触发条件**：`prim` 模块增长到需要自己的依赖（如涉及 fs 事务语义）时拆出。

### D8 不引入 protobuf 或任何编码变更

完整论证在 `unify-ipc-protocol-home` 的 design（含被否备选与重启触发条件）。此处只记结论：复制问题的成因是**可见性**而非**序列化**，换编码修不了它。

### D9 零动作清单（已核查，避免无谓改动）

- watchdog 控制 RPC（`ControlEnvelope` 等）与 `src/ipc.rs`（`ChildMsg`）：类型已单一归属根 crate，其全部客户端（CLI、standalone webui、im）都在根 crate 内，**不存在复制**，不动。
- `sebas-node-link`：已是正确的共享契约叶子 crate（两侧共用、禁止复制、独立 `PROTOCOL_VERSION`），保持现状。

## Risks / Trade-offs

- **[六份会话键实现并非全部等价]** → 这是本 change 最容易被做错的地方。`src/agent_backend.rs:331` 的 `encode_key` 用的是 `serde_json::to_string(key)`，与前五份的 `channel\0reference` **是不同编码**；`src/node_link/projection.rs:58` 还叠了一层 `node\0{node_id}\0{session_id}` 的嵌套方案。收敛时**必须逐点核对每处的键是否跨进程、是否被持久化、是否作为 map key 参与比较**：等价的才收敛，不等价的保留原行为并写明理由。验收靠「用收敛前的实现输出做黄金样本」逐字节比对，而不是靠眼看的「看起来一样」。
- **[domain 退化成垃圾抽屉]** → 准入规则写进 spec（只有 ≥2 个 crate 需要、且角色中立的类型可进），并在 review 时按规则驳回。拒收清单示例：`SessionRow`（展示）、`HostedSession`（节点内部）、`ProviderProfile`（节点自有语义）、watchdog 的 `OperationStatus`（服务监督域，与 session 无关）。
- **[叶子属性被无意破坏]**（有人在 domain 里 `use rusqlite` 或引入角色 crate）→ 加机械断言（tasks 2.6），让违规在 CI 而不是在半年后被发现。
- **[搬迁面广、机械但易漏]** → 编译器兜底（改的是定义出处，import 未更新即编译失败）+ 逐 crate 分步 + `pub use` 再导出保证调用点不变。
- **[零变化难以自证]** → 以既有 `invoke testsuite-e2e` / `invoke testsuite-acceptance` 全绿 + 线格式快照零差异为闸门；本 change 不引入新的对外行为，因此不需要新的验收套件，只需要既有套件不被打破。

## Migration Plan

纯重构，无数据迁移、无部署顺序要求、无灰度。

1. 建 `sebas-domain` 骨架并接进 workspace，确认空 crate 可编译、依赖图合规。
2. 收敛中立原语（会话键、`expand_tilde`、`now_unix`），每收敛一处即替换全部调用点并删重复；逐处跑相关 crate 测试。
3. 分 crate 搬中立契约类型（dispatch → webui `session_backend` → provider 状态词表），每步原 crate 加 `pub use`，跑该 crate 测试。
4. 收口显式重声明（`NodeView`/`NodeInfo`、`ModelAliasEntry`、overlay 读取器），并补形状钉测试。
5. 全量回归：`invoke testsuite-e2e` + `invoke testsuite-acceptance` + 线格式快照比对。

**回滚**：每一步都是独立可 revert 的提交，且不涉及数据格式变更，故回滚 = revert 该步；不存在需要回滚的持久化状态。若第 3 步中途发现某类型不适合共享（依赖反向），中止该类型的搬迁并把发现记入本文件——不影响已完成的步。

## Open Questions

- `sebas-node-link` 里的 `SessionSummary` / `SessionMode` / `ApprovalDecision` 与 `sebas-domain` 的对应概念存在**词汇重叠**（今日各自成立，因为两侧独立发布）。是否归并、还是让 node-link 只保留链路专属类型，留给 `type-session-vocabularies` 决策——它不改变本 change 的 spec、做法或任务分解。
- `sebas-domain` 的模块划分（`session` / `project` / `provider` / `prim` 还是一个扁平层）属于实现便利，可在落地时按实际体量定，不影响已写下的要求。
