## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **`sebas-ipc` 今天只有 135 行纯传输**：`bind` / `connect` / `accept` / `split`（`sebas-ipc/src/lib.rs`），文档明说「应用层 secret 握手由各通道协议自带」；它只依赖 `interprocess` + `tokio`，是干净叶子。`sebas-router` 已依赖它。
2. **协议类型的位置**：core channel 的 wire 类型在根 crate（`src/core_channel/protocol.rs`），**引用 dispatch 与 webui 的角色类型**（`:29` `use sebas_dispatch::{SessionInfo, TurnEntry, …}`，`:33` `use sebas_webui::session_backend::{PermissionDecision, PermissionNotice}`），而 `sebas-dispatch → sebas-router`、`sebas-webui → sebas-router` 两条边已存在。
3. **`node-session-channel` 已有完整演进机制**：`PROTOCOL_VERSION: u32 = 1`、逐字段比较、`HelloAck` 带 master 的 router 凭据、`CapabilityManifest` 能力协商、`RejectCode` / `SessionRejectCode` 用 `#[serde(other)] Unknown` 保前向兼容、`is_permanent()` 表达重试语义。
4. **core channel 无版本字段**：兼容性靠 `#[serde(default)]` 与「只增不改」的默契，外加一次已记录的破坏性变更（`Spawn` 的旧 `backend` 字段改为拒绝）。
5. **router 的重复是精确可指的**：`sebas-router/src/core_channel.rs:239` 重声明 `StateStreamFrame` 子集、`:223-233` 本地 `SnapshotResp`、内联 `json!({"cmd":"state_subscribe"})`，以及自己从 `SEBAS_ROUTER_CONFIG` 目录解析 secret（`:25-52`）。
6. **protobuf 状况**：workspace 无一处使用。`prost 0.13.5` 只在 `Cargo.lock` 里作为 `openlark`（飞书 WS）的传递依赖存在；`tonic` / `prost-build` 缺席。本机 `protoc` 是 **36.1**。

## Goals / Non-Goals

**Goals:**

- 让「同一协议被声明两遍」在结构上不可能（不是靠注释承诺）。
- 让 core channel 具备与 node link 同级的**演进机制**——不兼容时指名双方版本，而不是报一句解码失败。
- 把前向兼容从默契变成**机械闸门**。

**Non-Goals:**

- 不改编码（见 D2）。
- 不给 core channel 加能力协商（只补版本）。
- 不统一 framing（NDJSON 与 WS 文本帧各有理由）。
- 不触碰 watchdog 控制 RPC 与 `src/ipc.rs`（无复制，见 D8）。

## Decisions

### D1 协议之家 = `sebas-ipc`（transport 与协议同 crate）

- **理由**：握手、secret、framing 是 transport 与协议的交界，分居两 crate 会让「谁负责握手」永远模糊；`sebas-ipc` 已是干净叶子且 router 已依赖它，加协议不新增依赖边。与用户原话「跨进程协议统一定义到 sebas-ipc 里」一致。
- **被否备选**：新建 `sebas-proto`（更纯的单一职责，transport 与协议解耦）。**触发条件**：协议面膨胀到需要独立版本节奏，或需要支持跨语言 codegen 时再拆。

### D2 **不引入 protobuf**——复制问题的成因是可见性，不是序列化

论证按权重排列：

1. **浏览器是一等消费者**。52 个 wire 类型里约 45 个属 webui HTTP/WS/SSE 或 core-channel NDJSON。SPA 直接 `JSON.parse`，`ws_rpc_contract_test.rs` 断言 JSON 字段名。在那里上 protobuf **不是省一层而是加一层**：要么前端引 protobuf-js（新工具链、新产物体积），要么在 axum 边缘加 proto↔JSON 转码——而 253 处 `json!` 字面量无论编码如何都还在。
2. **protobuf 的回报来自「独立发布 × 跨语言」的消费者**。本系统只有一个真正独立发布的边界：`sebas-node`，且它**已经**有版本号与能力协商。core / webui / im / router / CLI 是同一 workspace、同一提交锁步编译，其中 `src/run.rs` 还把 webui 跑在 core 进程内。
3. **可读的原始字节在本仓库是承重结构**。AGENTS.md 的沙箱菜谱教 agent 读 `sebas router listening addr=…` 并断言 NDJSON 握手；fake-provider 的 journal 是 NDJSON；e2e 套件断言 JSON 形状。「线上是 JSON」目前是特性。
4. **要治的病换编码治不了**。10 处显式重声明的成因**全部**是可见性（根 crate 不可依赖、`pub(crate)` 够不着、循环依赖）。搬进一个叶子 crate 治好 100%；protobuf 治好 0%——你仍然需要那个 crate，而 prost 生成的类型不携带 `#[serde(other)] Unknown` 这种仓库在用的前向兼容表达，也不带 `#[serde(default)]` 的只增语义。
5. **成本真实存在**，即便本机有 `protoc 36.1`：每 crate 的构建期 codegen、若要保 JSON 还要 `pbjson`、生成代码进入 `cargo tree -p sebas-node` 的依赖纪律核对、以及「构建依赖 protoc 存在于每台开发机与 CI」——与 AGENTS.md 刻意保持封闭的沙箱菜谱相冲突。

**用 protobuf 想换到的那件事（字段号演进而非名字演进），有更便宜的等价物，而且现在就能拿到**：core channel 补 `version` + 协商（D7）、每个 wire 枚举保留未知值路径（D5）、只增不改写成规则、golden fixture 机械守住（D6）。`node-link` 已经证明了这套模式跑得通。

**重启触发条件**（三者同时成立才重新评估，且届时只对 node-link 那一条边界用 `.proto` + `pbjson` 保持线上 JSON）：① node-link 出现独立发布节奏；② 载荷显著增长（大 transcript、material 文件传输）；③ 出现非 Rust 的消费者。**不满足就不要做。**

### D3 协议 crate 只依赖中立叶子——这是 change 1 解开的环

`src/core_channel/protocol.rs` 引用 dispatch 与 webui 的类型（Context 2）。若协议 crate 照搬这些依赖，则 router 依赖它即得 `router → protocol → webui/dispatch → router`（Context 2 的两条边都已存在）。因此协议 crate 的依赖只能是 `sebas-domain` / `sebas-channels` 一类叶子——**这就是 `add-domain-layer` 必须排在前面、且把中立契约类型先搬出去的原因**。该约束写进 spec（`ipc-protocol-home`「The protocol crate depends only on neutral leaves」）并由机械断言守住。

### D4 router 删除自备协议

`sebas-router` 的 `StateStreamFrame` 子集、本地 `SnapshotResp`、内联 `json!` 请求、以及自解析 secret 全部换成共享实现。**收益**：core 侧改一个字段名，router 直接编译失败——今天它只会静默不匹配。**风险**：router 的 hot-reload 行为可能因握手路径变化而变 → 用既有 router 测试（含 `process_e2e_test` / `contract_test`）钉住。

### D5 前向兼容规则书面化

- 新增字段**必须**带 serde 默认值（今天多处已这么做，但没写下来）。
- 取 wire 值的枚举**必须**保留未知值路径（`RejectCode` 的 `#[serde(other)] Unknown` 是范例；裸字符串枚举在 `type-session-vocabularies` 里已定同款规则）。
- 字段删除与改名 = **破坏性变更**，必须在 change 里显式声明（今天只有一次这样的记录，是好事，写进规则）。

### D6 golden fixture 是闸门，不是文档

每条边界一份 checked-in fixture：① 代表性载荷的**序列化字节**；② 该消息的**字段名集合**。两者都比对，所以删字段、改字段名必红，而新增带默认值的字段不红（正确语义）。fixture 只在**有意的**破坏性变更里更新，且该 change 必须说明缘由。

### D7 版本协商 additive，且不得削弱既有认证次序

- 握手中的版本字段带默认值：**缺版本视作版本 1**，旧客户端读不到新字段也能解码。
- 未来版本 → 类型化拒绝，**同时指名客户端版本与本端支持版本**（对齐 `node-link` 的 `ProtocolVersionUnsupported` 行为）。
- **次序**：Unix peer-uid → shared secret → 版本协商 → 请求处理。版本不支持不得先于认证泄露给未认证连接（spec 的第四个场景把这条钉住）。
- **不给 core channel 加能力协商**：node-link 的能力清单解决的是「节点能跑哪些 agent kind」这类真实分支需求，core channel 今天没有等价需求；没有需求的能力协商只是负担。触发条件：出现真实的按能力分支。

### D8 零动作清单

- watchdog 控制 RPC：**已有** `version: u16` 且只接受 1、secret 校验、socket 0600，其全部客户端（CLI、standalone webui、im）都在根 crate 内——**无复制**，不动。
- `src/ipc.rs`（`ChildMsg`，watchdog↔core 子进程就绪信号）：客户端同为根 crate，无复制，不动。
- `sebas-node-link`：已是正确的共享契约 crate，只纳入 golden fixture，不改结构。
- webui 面向浏览器的契约：不是内部 IPC，不改。

## Risks / Trade-offs

- **[握手加字段改变线上字节]** → additive + 默认值；两个方向各一个用例（新服务端 × 旧客户端、旧服务端 × 新客户端）。这是本 change 唯一有意的 wire 变化，必须在 change 记录里点名。
- **[认证次序被版本检查插到前面，导致向未认证连接泄露版本支持]** → D7 的次序约束 + 专场景覆盖（错 secret + 不支持版本 → 只报认证失败）。
- **[搬协议类型时漏看它引用的角色类型]** → 编译器兜底：proposed 搬迁后若仍引用 dispatch/webui，`cargo build -p sebas-ipc` 立刻失败；再加机械断言（公开面与依赖图都不含角色实现）。
- **[router 改复用后 hot-reload 行为漂移]** → 既有 router 测试套件（`contract_test` / `process_e2e_test` / `hot_reload` 相关）不改而全绿的硬要求。
- **[golden fixture 变成维护负担]** → 只在有意破坏性变更时更新，且更新必须写理由；新增带默认值字段不需要动 fixture（语义正确，不是漏洞）。
- **[`sebas-ipc` 变成什么都往里塞的仓库]** → spec 要求公开面不得含角色实现；crate 文档写明准入面（transport + wire 类型 + 握手/framing 助手），与 `sebas-domain` 的准入规则并列。

## Migration Plan

顺序（依赖已满足：`add-domain-layer` 已实现归档，中立契约类型已居 `sebas-domain`）：

1. `sebas-ipc` 增加 protocol 模块与握手/framing 助手；接进 workspace 依赖；机械断言其依赖图不含角色实现。
2. core channel 的 wire 类型从根 crate 迁入 `sebas-ipc`，根 crate 原位再导出（调用点零改动）；确保不再引用 dispatch/webui 类型。
3. core channel 握手加版本字段 + 协商逻辑 + 类型化版本拒绝；补双向兼容用例与认证次序用例。
4. `sebas-router` 删自备协议，改复用共享定义；跑 router 全套测试。
5. 四套 golden fixture 落地（core channel / node link / webui WS / webui HTTP），含字段名集合比对。
6. 前向兼容规则核对：逐条检查既有 wire 枚举是否都有未知值路径，缺的补齐。
7. 全量回归：`invoke testsuite-e2e` + `invoke testsuite-acceptance`。

**回滚**：分步提交。第 3 步（唯一的 wire 变化）单独一个提交，回滚它即回到「无版本字段」的兼容状态；fixture 与 spec 同提交，不留下悬空契约。

## Open Questions

- `sebas-ipc` 是否需要独立的版本号（今天随 workspace 一起版本）：只有当它被独立发布时才需要，今日无此需求。不影响本 change 的 spec、做法与任务分解。
- core channel 是否最终也要能力协商：见 D7 的触发条件，待真实需求出现。
- `sebas-node-link` 是否改名为统一前缀（如 `sebas-ipc-node-link`）：纯命名，无行为影响，不在本 change。
