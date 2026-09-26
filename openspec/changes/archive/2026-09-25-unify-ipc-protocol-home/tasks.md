## 1. sebas-ipc 成为协议之家

- [x] 1.1 在 `sebas-ipc` 增加 `protocol` 模块骨架与握手/framing 助手，依赖加入 `sebas-domain` / `sebas-channels`（仅叶子）；验证：`cargo build -p sebas-ipc` 通过，且 `cargo tree -p sebas-ipc` 不含任何角色实现（core / webui / router / im）与 sebas-node
  - 证据：新增 `sebas-ipc/src/protocol.rs`（`WireFrame` trait + blanket impl 的 `to_line`/`from_line`、`encode_line`/`decode_line`、`Attachment`/`marker()`、`PROTOCOL_VERSION`、`ChannelHandshake`/`ChannelHandshakeAck` + `negotiate()`）与 `sebas-ipc/src/secret.rs`（`ChannelSecret` 共享发现实现）；`sebas-ipc/Cargo.toml` 增 `serde`/`serde_json`/`tracing`/`sebas-domain`/`sebas-channels`。
  - `cargo build -p sebas-ipc` → `Finished dev profile`，0 error 0 warning。
  - `cargo tree -p sebas-ipc --prefix none --depth 1` → 仅 `interprocess` / `sebas-channels` / `sebas-domain` / `serde` / `serde_json` / `tokio` / `tracing` / `tempfile(dev)`；全树 `awk '{print $1}' | sort -u` 与 `^(sebas|sebas-webui|sebas-router|sebas-im|sebas-node)$` 取交集为**空**（命令退出码 1 = 无匹配）。机械断言固化在 `tests/ipc_protocol_home_test.rs:sebas_ipc_depends_on_no_role_implementation`。

- [x] 1.2 加机械断言测试（放在根 crate 集成测试）：解析 `cargo tree -p sebas-ipc` 与 `sebas-ipc` 公开符号，断言不含角色实现、不含域表名；验证：新增测试通过，且临时给 `sebas-ipc` 加一条 `sebas-webui` 依赖时该测试失败（附一次失败演示）
  - 证据：新增 `tests/ipc_protocol_home_test.rs`（6 个测试）——`sebas_ipc_depends_on_no_role_implementation`（解析 `cargo tree`）、`neutral_leaves_are_present_and_node_link_is_not_mistaken_for_the_node`、`the_channel_speakers_reach_the_protocol_crate_by_path_dependency`、`sebas_ipc_public_surface_names_no_role_crate_and_no_domain_table`（扫 `sebas-ipc/src`，禁 `sebas_dispatch|sebas_webui|sebas_router|sebas_im|sebas_feishu|sebas_node::|sebas_models|sebas_state` 与域表名字面量 `"projects"`/`"session_map"`/`"model_aliases"`/`"users"`/`"usage_records"`/`"schema_meta"`）、`root_crate_re_exports_the_protocol_instead_of_declaring_it`、`router_carries_no_private_copy_of_the_channel_protocol`。`cargo test --test ipc_protocol_home_test` → `6 passed; 0 failed`。
  - **失败演示**：向 `sebas-ipc/Cargo.toml` 临时追加 `sebas-webui = { path = "../sebas-webui" }` 后
    `cargo test --test ipc_protocol_home_test sebas_ipc_depends_on_no_role_implementation` →
    `FAILED ... panicked at tests/ipc_protocol_home_test.rs:109: sebas-ipc 的依赖图出现了主控角色/执行节点实现 ["sebas-webui", "sebas-router"]`（连 webui 带进来的 router 也一并抓到）。恢复 `Cargo.toml` 后同一测试通过。

- [x] 1.3 crate 文档写明准入面（transport + wire 类型 + 握手/framing 助手）与「不得含角色实现」纪律；验证：`cargo doc -p sebas-ipc` 无警告，文档含准入清单
  - 证据：`sebas-ipc/src/lib.rs` crate 文档新增「准入面」小节（transport / wire 类型 / 握手与 framing 助手三块清单）与「不得含角色实现」纪律（禁角色 crate、禁域表名、禁 secret 文件生命周期），并写明三条前向兼容规则；`sebas-ipc/src/protocol.rs` 模块文档写明 framing 即兼容面。
  - `cargo doc -p sebas-ipc --no-deps` → `warning` 计数 **0**（顺手修掉一处既有 broken intra-doc link：`IpcListener::accept` → `accept`，`sebas-ipc/src/lib.rs:9`）。生成物 `target/doc/sebas_ipc/index.html` 中「准入面」出现 3 次。

## 2. core channel 协议类型迁出根 crate

- [x] 2.1 把 `CoreChannelRequest` / `CoreChannelResponse` / `SessionStreamFrame` / `StateStreamFrame` / `ChannelHandshake` / `Attachment` / `NodeLinkOp` / `NodeLinkOutcome` / `NodeView` 迁入 `sebas-ipc::protocol`，根 crate 原位 `pub use`；验证：`cargo test -p sebas` 全绿，且 `src/core_channel/protocol.rs` 不再定义这些类型
  - 证据：`src/core_channel/protocol.rs` 现为薄壳 `pub use sebas_ipc::protocol::{Attachment, ChannelHandshake, ChannelHandshakeAck, CoreChannelRequest, CoreChannelResponse, NodeLinkOp, NodeLinkOutcome, NodeView, PROTOCOL_VERSION, SessionStreamFrame, StateStreamFrame, WireFrame, decode_line, default_protocol_version, encode_line};`（430 行 → 只剩再导出与原位保留的 wire-shape 测试模块）。定义处：`sebas-ipc/src/protocol.rs`。
  - 根 crate 全量测试（`env -u ANTHROPIC_*` 后 `cargo test`）→ `passed=702 failed=0`；`src/core_channel/protocol.rs` 中已无 `pub enum CoreChannelRequest` / `pub struct ChannelHandshake` 等定义（机械断言 `tests/ipc_protocol_home_test.rs:root_crate_re_exports_the_protocol_instead_of_declaring_it`）。

- [x] 2.2 确认迁出后不再引用角色 crate 类型（依赖 change 1 已把中立契约类型搬到 `sebas-domain`）；验证：`cargo build -p sebas-ipc` 通过且 `grep -n "sebas_dispatch\|sebas_webui" sebas-ipc/src/` 无输出
  - 证据：`sebas-ipc/src/protocol.rs` 改从 `sebas_domain::session::*` / `sebas_domain::node::NodeView` / `sebas_domain::vocabulary::*` / `sebas_channels::ChannelKey` 取类型（原 `sebas_dispatch` / `sebas_webui` 引用全部消除）。
  - `cargo build -p sebas-ipc` 通过；`grep -rn "sebas_dispatch\|sebas_webui" sebas-ipc/src/` → **无输出**（退出码 1）；同口径断言在 `sebas_ipc_public_surface_names_no_role_crate_and_no_domain_table` 中常驻。

- [x] 2.3 复核 wire 形状零变化：对迁出前后的同一载荷做序列化比对；验证：逐字节一致（含 `cmd` / `frame` / `op` / `result` 标签与字段名）
  - 证据：`tests/fixtures/ipc_core_channel_wire.json` 的 `payloads` 是**迁出前**由原代码序列化出的真实字节，冻结 17 条代表性载荷（7 条请求含 `cmd`/`op` 标签、5 条响应含 `cmd`/`result` 标签、5 条帧含 `frame` 标签），每条同时记 `bytes` 与顶层 `fields`。
  - 逐条比对结果：**17/17 逐字节一致**（`cargo test --test ipc_protocol_contract_test core_channel_payloads_are_byte_identical_to_the_pre_migration_fixture` → ok）。全量四套 fixture 比对中唯一字节差异是 `handshake`（迁出前 `{"secret":"s3cret"}` → 现 `{"secret":"s3cret","version":1}`），即 7.1 声明的那一处有意变化，其余零差异。

## 3. core channel 握手版本协商

- [x] 3.1 `ChannelHandshake` 增加带默认值的版本字段，服务端握手响应携带本端版本；验证：单测断言字段缺省时按版本 1 处理，且序列化含版本字段
  - 证据：`sebas-ipc/src/protocol.rs` 的 `ChannelHandshake { secret: String, #[serde(default = "default_protocol_version")] version: u32 }`；`PROTOCOL_VERSION: u32 = 1`。单测 `handshake_without_version_is_treated_as_version_one`（`{"secret":"s3cret"}` 解码后 `version == 1`）与 `handshake_serializes_the_version_field`（序列化含 `"version":1`）。
  - 服务端侧：`src/core_channel/server.rs::write_handshake_ack` 回 `{"handshake":"ok","version":1}`；根 crate 单测 `server_ack_carries_the_local_protocol_version` 断言 `v["handshake"]=="ok"` 且 `v["version"]==PROTOCOL_VERSION` → ok。

- [x] 3.2 版本不匹配 → 类型化拒绝，指名客户端版本与本端支持版本；验证：单测断言拒绝文本含两个版本号，且该连接上无请求被处理
  - 证据：`ChannelHandshakeAck::VersionUnsupported { version, client_version }` → wire `{"handshake":"version_unsupported","version":1,"client_version":9}`（**两个版本号都上 wire，机读**），`cause()` 产出 `... client=9 supported=1`。
  - 单测（`src/core_channel/tests.rs`）：`unsupported_version_is_typed_rejected_and_no_request_is_processed`（断言 `cause` 含 `client=9` 与 `supported=1`，且 JSON 里 `client_version==9`、`version==1`）与 `no_request_is_served_on_a_version_rejected_connection`（先收到版本拒绝、随后再读是 **EOF**，证明请求未被处理）→ 均 ok。

- [x] 3.3 认证次序不被削弱：peer-uid → secret → 版本 → 请求；验证：专项用例「错 secret + 不支持的版本」只报认证失败、不泄露版本支持（PostgreSQL 风格的 fail-closed 次序）
  - 证据：`src/core_channel/server.rs::handle_connection` 严格按 peer-uid（Unix，`cross_uid_rejected_live_process`）→ secret 比对（不等则**直接 return，一个字节都不回**）→ 版本协商 ack → 请求循环；`read_handshake` 不再比对 secret，`write_handshake_ack` 只在校验通过后调用。
  - 专项单测 `wrong_secret_with_unsupported_version_discloses_nothing`：`{"secret":"totally-wrong","version":9}` → 服务端读回 **EOF（无任何字节）**，绝不泄露版本支持；对照组「正确 secret + version 9」确实收到 `version_unsupported` → ok。

- [x] 3.4 双向兼容用例：①新服务端 × 不发版本的旧客户端 → 按 1 服务；②旧服务端 × 带版本字段的新客户端 → 靠默认值解码成功；验证：两个用例各自通过，附握手原文
  - 证据：①根 crate 单测 `legacy_client_without_version_is_served_as_version_one`，握手原文 `{"secret":"<SECRET>"}` → 收到 `{"handshake":"ok","version":1}`，且随后请求被正常应答（真的按 1 服务）→ ok。
  - ②`sebas-ipc::protocol` 单测 `old_server_ack_without_version_decodes_with_default`：旧服务端应答原文 `{"handshake":"ok"}` → `ChannelHandshakeAck::from_line` 解出 `Ok { version: 1 }`（靠 `#[serde(default)]`）→ ok。
  - 第三个方向（新客户端 × 新服务端）由 `client.rs::handshake` 的 `Ok(Ok { version }) if version <= PROTOCOL_VERSION` 分支与 `sebas-ipc` 单测 `handshake_ack_carries_the_local_version` / `future_version_is_rejected_naming_both_versions` 覆盖。

## 4. router 删除自备协议

- [x] 4.1 `sebas-router/src/core_channel.rs` 改为复用 `sebas-ipc` 的握手、帧类型与请求构造，删掉本地 `StateStreamFrame` 子集（`:239`）、本地 `SnapshotResp`（`:223-233`）与内联 `json!` 请求；验证：`cargo test -p sebas-router` 全绿，且 `grep -n "StateStreamFrame\|SnapshotResp" sebas-router/src/` 只剩导入
  - 证据：本地 `enum StateStreamFrame`（原 `:239`）、`struct SnapshotResp`（原 `:223-233`）、`json!({"secret":...})` / `json!({"cmd":...})` 请求与 `json!` 响应解析全部删除；`channel_request` 改签名为 `(&Path, &CoreChannelRequest) -> Result<CoreChannelResponse, String>`，用 `ChannelHandshake::new(..).to_line()` / `ChannelHandshakeAck::from_line` / `req.to_line()` / `CoreChannelResponse::from_line`；`fetch_state_snapshot` 用 `CoreChannelRequest::StateSnapshot{domain}` 并 `match CoreChannelResponse::StateSnapshot`；`subscribe_once` 用 `CoreChannelRequest::StateSubscribe` + `StateStreamFrame::from_line` 循环（`Snapshot`/`Changed`/`Unknown` 三臂）。
  - `grep -n "StateStreamFrame\|SnapshotResp" sebas-router/src/*.rs` → 只剩 `core_channel.rs:21` 的导入与 5 处**类型使用**（无定义、无本地副本）；`grep -n "json!"` → 仅 2 处**注释**。`cargo test -p sebas-router` → unit `213 passed` + 集成 `82 passed; 0 failed`（12 个 target 全绿）。

- [x] 4.2 secret 解析改为复用共享实现；验证：`sebas-router` 的既有 secret/鉴权用例不改而通过
  - 证据：`channel_secret()` 只保留**落点推导**（`SEBAS_ROUTER_CONFIG` 所在目录 + `sebas_ipc::secret::SECRET_FILE_NAME`），解析本身调 `ChannelSecret::from_env_or_file(file).current()`（`sebas-router/src/core_channel.rs`，`use sebas_ipc::secret::ChannelSecret;`）；本地复刻的 env→文件 逻辑删除。
  - `git diff --stat sebas-router/tests/` → **空**（既有 secret/鉴权用例一字未改）；`cargo test -p sebas-router --test contract_test --test process_e2e_test` → `16 passed` + `5 passed`，含 `real_binary_forwards_anthropic_openai_auth_and_usage` 全绿。

- [x] 4.3 用「core 侧改一个字段名 → router 编译失败」验证闸门生效；验证：临时改名后 `cargo build -p sebas-router` 失败，恢复后通过（附一次失败演示）
  - **失败演示**：把 `sebas-ipc/src/protocol.rs` 的 `StateStreamFrame::Changed { scope: String }` 临时改名为 `{ scopes: String }` 后
    `cargo build -p sebas-router` →
    ```
    error[E0026]: variant `Changed` does not have a field named `scope`
      --> sebas-router/src/core_channel.rs:311:41
    311 |             StateStreamFrame::Changed { scope } => {
    help: a field with a similar name exists
    311 |             StateStreamFrame::Changed { scopes } => {
    ```
    （`could not compile sebas-router`）。恢复后 `cargo build -p sebas-router` → `Finished dev profile`。
  - 意义：改名之前 router 只会静默不匹配（自备子集各自解析），现在**编译期**就断——这正是 4.1 要买的性质。

- [x] 4.4 router 行为回归：hot-reload 与 state 订阅路径；验证：`cargo test -p sebas-router` 含 `process_e2e_test` / `contract_test` 全绿
  - 证据：`cargo test -p sebas-router` → 12 个 target 全绿（unit `213`、`admin_test` 13、`auth_test` 9、`contract_test` 16、`debug_provider_test` 10、`failure_test` 7、`process_e2e_test` 5、`proxy_smoke_test` 5、`rate_limit_test` 6、`server_smoke_test` 5、`spec_diff_test` 6）；`0 failed`。
  - 活体复核（7.4 沙箱）另证：router 日志 `core channel subscribed, reloading providers` → `core channel snapshot applied, config hot swapped`，并在 provider 变更后收到 `core channel state change: scope=providers, reloading` 且 `/admin/stats` 的 `providers` 由 2 升到 3——hot-reload 与订阅路径都真的在跑。

## 5. golden fixture 契约闸门

- [x] 5.1 core channel fixture：代表性载荷（请求、响应、快照帧、事件帧、拒绝）的序列化字节 + 字段名集合；验证：fixture 测试通过，且临时删一个字段时测试失败（附一次失败演示）
  - 证据：`tests/fixtures/ipc_core_channel_wire.json`（17 条载荷 + `handshake.before/after`）+ 闸门 `tests/ipc_protocol_contract_test.rs:core_channel_payloads_are_byte_identical_to_the_pre_migration_fixture`（逐字节 + 字段名集合双断言）。`cargo test --test ipc_protocol_contract_test` → `6 passed; 0 failed`。
  - **失败演示**：临时从 `Attachment` 删掉 `mime` 字段后
    `cargo test --test ipc_protocol_contract_test core_channel_payloads_are_byte_identical` →
    ```
    FAILED ... panicked at tests/ipc_protocol_contract_test.rs:77:
    [core-channel] `request.message` 的序列化字节变了。删字段 / 改字段名 / 改枚举取值 /
    新增会序列化的字段，都是 wire 形状变化：必须先在 change 里声明、说明迁移路径，再有意更新 fixture。
    当前: ..."attachments":[{"path":"/tmp/img.png","name":"img.png"}]
    fixture: ..."attachments":[{"path":"/tmp/img.png","mime":"image/png","name":"img.png"}]
    ```
    恢复后同一测试 ok。

- [x] 5.2 node link fixture 覆盖握手与版本协商消息；验证：fixture 测试通过，且既有 `PROTOCOL_VERSION` 逐字段比较与「指名双方版本」的拒绝行为不变
  - 证据：`tests/fixtures/ipc_node_link_wire.json` 钉住 `hello.join_token` / `hello_ack.accepted` / `hello_ack.rejected` 三条消息的字节与字段集合；闸门 `node_link_handshake_messages_match_the_fixture` → ok。
  - `node_link_version_rejection_still_names_both_versions` → ok：断言 `PROTOCOL_VERSION == 1`（本 change 未动链路版本）、`outcome.result=="rejected"`、`outcome.code=="protocol_version_unsupported"`、成因同时含节点版本 `9` 与主控支持版本 `1`；并复核未知拒绝码仍走 `#[serde(other)] Unknown` 且 `is_permanent()==true`（前向容错未被削弱）。与既有 `sebas-node-link/src/golden_tests.rs` 互补（那边钉词汇，这边钉握手/版本协商）。

- [x] 5.3 webui WS 与 HTTP fixture；验证：与既有 `ws_rpc_contract_test.rs` 不冲突、两套都通过，HTTP 侧形状与 `api_endpoints_test` 断言一致
  - 证据：`tests/fixtures/ipc_webui_wire.json` 钉住 4 条 WS 帧（request / response.ok / response.error / notification）与 3 条 HTTP 载荷（summary / session_row / rejection_body）；闸门 `webui_ws_frames_match_the_fixture` 与 `webui_http_payload_shapes_match_the_fixture_and_api_assertions` → ok。
  - 不冲突验证：`cargo test -p sebas-webui --test ws_rpc_contract_test --test api_endpoints_test` → `22 passed` + `5 passed`，两套全绿；`git diff --stat sebas-webui/tests/` 为**空**（既有断言一字未改）。HTTP 侧形状与 `api_endpoints_test` 口径对齐（`reachability.ok`、`execution_bodies.<name>.ok` 均为布尔真值，测试内显式复核）。

- [x] 5.4 复核「新增带默认值字段不触发 fixture 失败」；验证：临时加一个带默认值的字段，fixture 测试仍通过（证明闸门语义正确，不是全量卡死）
  - **复核**：临时给 `Attachment` 加 `#[serde(default, skip_serializing_if = "Option::is_none")] pub sha256: Option<String>`（缺省值不上 wire 的 additive 形态）→ `cargo test --test ipc_protocol_contract_test` **仍 6 passed / 0 failed**（无需动 fixture）。已恢复。
  - 闸门语义说明（已写进 `assert_pinned` 文档）：字节断言钉住已钉载荷的完整形状；字段名断言是**子集**断言（fixture 字段必须都还在）——所以「删字段/改字段名」两条都红，而**兼容的增列**（带默认值、缺省不上 wire）两条都过。反过来，新增一个**会序列化**的字段确实会红——那本来就是一次 wire 形状变化，必须先声明（与握手加版本字段同理），不是闸门误报。

## 6. 前向兼容规则核对

- [x] 6.1 逐条核对既有 wire 枚举是否都有未知值路径（对齐 `node-link` 的 `#[serde(other)] Unknown` 范例），缺的补齐；验证：产出一张「枚举 → 未知值路径」清单，清单无空缺
  - 证据：交付物 `openspec/changes/unify-ipc-protocol-home/audit-enum-unknown-paths.md`——四张表（core session channel / node link / webui↔浏览器 / D8 零动作清单）逐条给出「`other` 变体 / 类型化解码失败 / 排除（附理由）」三档口径，无空缺。
  - **补齐 9 个 enum**：`CoreChannelRequest`、`CoreChannelResponse`、`SessionStreamFrame`、`StateStreamFrame`、`NodeLinkOp`、`NodeLinkOutcome`（`sebas-ipc/src/protocol.rs`）、`ChannelHandshakeAck`（本 change 新生）、`SessionRejection`（`sebas-domain/src/session.rs`，新增 `#[serde(other)] Unknown`）。每个消费者都补了明确分支，**没有一条是「忽略掉、不处理」**：`server.rs` 的请求分发回类型化拒绝、`client.rs` 的流循环保持连接健康、`sebas-router` 的订阅循环忽略未知帧并留住流、`node_link_cmd.rs` 把未知结果按**失败**呈现（非零退出）、`sebas-webui/src/api.rs` 把未知拒绝映射 502（不把「我不认识」说成「你的请求有问题」）。
  - **诚实记录的 2 处遗留缺口**：`PendingDisposition` / `PendingReason` 是普通（非 tagged）unit 枚举，serde 的 `#[serde(other)]` **不适用**，补齐需手写 `Deserialize`；已具名、给出理由与触发条件（core channel 出现真实按版本分支需求时先补），未静默略过。

- [x] 6.2 核对每条边界的新增字段是否都带 serde 默认值；验证：抽查三条边界的近期字段，均带默认值
  - 证据：机械抽查 `Option<..>`/`Vec<..>` 字段是否带 `serde(default)`：
    - `sebas-ipc/src/protocol.rs` → `defaulted=2 missing_default=0`（`Attachment.mime` / `Attachment.name`；`ChannelHandshake.version` 走 `default = "default_protocol_version"`）。
    - `sebas-node-link/src/lib.rs` → `missing_default=0`（`Hello.manifest`、`HelloAck.credential`/`router_url`/`router_token`、`NodeView.last_seen_unix` 等全部带默认值）。
    - `sebas-webui/src/ws_rpc.rs` → `missing_default=0`（`RequestFrame.params`、`ResponseFrame.result`/`error`）。
    - 唯一命中 `missing_default=2` 的 `sebas-webui/src/events.rs` 的 `WebUiEvent` 是 **`Serialize`-only**（`#[derive(Debug, Clone, Serialize)]`，无 `Deserialize`，只出不进），不适用本规则——已记入 6.1 清单的「排除」档。

- [x] 6.3 把「新增字段须带默认值 / 枚举须留未知路径 / 删改属破坏性变更」写进 `AGENTS.md` 的协作约定；验证：文档含三条规则且与 `specs/ipc-protocol-home/spec.md` 一致
  - 证据：`AGENTS.md` 新增「### 协议演进三规则（unify-ipc-protocol-home 6.3）」小节（在 crate 速查表/准入规则之后的协作约定区，紧邻原「Frontend/Backend Integration Testing」之前，未重排周边章节），三条规则逐条对应 spec「Forward compatibility rules are explicit and enforced」的三条；并指向机械闸门 `tests/ipc_protocol_contract_test.rs` 与 6.1 清单。同时在 crate 速查表补一行 `sebas-ipc` 定位与准入（含机械断言文件指针）。

## 7. 全量回归与收口

- [x] 7.1 记录本 change 唯一的有意 wire 变化（握手新增版本字段）并在 spec 与文档点名；验证：`proposal.md` 与 `design.md` 均有该条目，且无其它 wire 差异
  - 证据：`openspec/changes/unify-ipc-protocol-home/proposal.md` 的「有意的 wire 变化（additive，本 change 唯一一处）」条目写明握手帧 `{"secret":…}` → `{"secret":…,"version":1}`、应答加 `version`、版本不受支持时 `{"handshake":"version_unsupported","version":N,"client_version":M}`，并给出机械证据指针；`design.md` 的「[握手加字段改变线上字节]」条目同样点名三处实现落点。
  - 「无其它 wire 差异」的机械证据：`tests/fixtures/ipc_core_channel_wire.json` 的 17 条迁出前载荷逐字节一致 + 四套 fixture（core channel / node link / webui WS / webui HTTP）全绿；`cargo test --test ipc_protocol_contract_test` → `6 passed; 0 failed`。全量比对中唯一差异载荷 = `handshake`。

- [x] 7.2 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
  - 证据：`invoke testsuite-e2e` → `✅ e2e — 74 passed · 40.4s [cargo]`，`test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out`。报告 `.artifacts/verify/report-e2e.html`。（前置：`cargo build -p sebas-node` 与 `cargo build -p sebas-acp`，否则依赖 crate 的 bin 未构建会导致假红。）

- [x] 7.3 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位并记录
  - 证据：`invoke testsuite-acceptance` → `✅ acceptance — 10 passed · 12.7s [cargo]`，`test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out`（`model_and_provider` 3、`workbench_and_projects` 3、`session_and_turn` 2、`remote_node` 2）。报告 `.artifacts/verify/report-acceptance.html`。无红。

- [x] 7.4 跨角色联调复核：按 AGENTS.md 沙箱菜谱起 core（+webui）与独立 router，走一次会话创建与 provider 状态订阅；验证：`/api/summary` 的 `reachability.ok = true`、router 日志显示握手成功且订阅到状态帧
  - 证据：一次性沙箱 `/tmp/sebas-ipc-74`（`SEBAS_STATE_DIR` 单变量 + `SEBAS_STATE_FILE` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 钉住，`HOME` 钉住，**未设** `SEBAS_CORE_SECRET` —— core 自动武装并写 `core.secret`，router 经 `sebas_ipc::secret` 共享发现读到它）。core 起在 `--webui-port 9877`，独立 `sebas router -c … --debug`（`SEBAS_CORE_SOCKET` 指向沙箱 socket）。
    - `GET /health` → `ok`；`GET /api/summary` → `"reachability": {"ok": true}`，`execution_bodies` 中 `acp.ok = true`（`native.ok = false` 是沙箱无 `SEBAS_AGENT_PROVIDER_API_KEY` 的预期，如实记录）。
    - router 日志：`core channel subscribed, reloading providers` → `core channel snapshot applied, config hot swapped`（**握手成功 + 订阅到快照帧**）；随后 provider 变更时 `core channel state change: scope=providers, reloading` + 再次 `snapshot applied`，`/admin/stats` 的 `providers` 由 2 升至 3（**订阅到变更帧**）。
    - 会话创建：`POST /api/projects` → `proj-5c343f168d93`；`POST /api/sessions {"prompt":"hello","backend":"acp","agent":"claude","project_id":…}` → `{"key":"web%00web-…"}`；`GET /api/sessions/<key>` → `status_label: "Done"`，entries 为 `hello` + fake-claude 的 `hello world`（跨角色通道 + ACP 回合走通）。
    - 收尾：SIGTERM core → socket 被优雅删除（`core-channel.sock` 不存在）；9877/8787 已释放；沙箱目录已删；操作员真实实例（9797，pid 2010159）全程未触碰；主检出 `/data/workbench/repos-ai/sebas` 仍在 `c309370` 且 `git status` 干净。

## 状态备注

**25/25 全部完成**，所有闸门实跑并留下真实数字：

| 闸门 | 结果 |
|---|---|
| `cargo build` | 通过（仅 1 处既有无关 warning：`src/watchdog/executor.rs:113 field service is never read`） |
| `cargo build -p sebas-ipc` | 通过，0 warning |
| `cargo tree -p sebas-ipc` | 仅中立叶子 + 通用库；无角色实现、无 `sebas-node` |
| `cargo doc -p sebas-ipc --no-deps` | **0 warning** |
| `cargo test -p sebas-ipc` | 15 passed / 0 failed |
| `cargo test -p sebas-domain` | 48 passed / 0 failed |
| `cargo test -p sebas-router` | 213 unit + 82 集成 passed / 0 failed |
| `cargo test -p sebas-webui`（等 5 crate） | 862 passed / 0 failed |
| 根 crate `cargo test`（`env -u ANTHROPIC_*`） | **702 passed / 0 failed** |
| `cargo test --no-run` | 全部测试 target 编译通过（0 error） |
| `invoke testsuite-e2e` | **74 passed / 0 failed** |
| `invoke testsuite-acceptance` | **10 passed / 0 failed** |
| 7.4 沙箱跨角色联调 | `reachability.ok = true`；握手成功、订阅到快照与变更帧；会话回合走通 |

**三次失败演示（任务要求，均已实做并恢复）**：
1. **1.2** 给 `sebas-ipc` 加 `sebas-webui` 依赖 → 依赖图断言红（还连带抓到 `sebas-router`）。
2. **4.3** core 侧把 `StateStreamFrame::Changed.scope` 改名 → `cargo build -p sebas-router` 编译失败（E0026，精确指到 router 使用点）。
3. **5.1** 从 `Attachment` 删 `mime` 字段 → fixture 字节比对红（报出前后完整字节差异）。

**5.4 反向复核**：给 `Attachment` 加一个带默认值且缺省不上 wire 的字段 → fixture 测试**仍全绿**（闸门不是全量卡死，兼容增列无需动 fixture）。

**与 design 的偏差（如实记录）**：
- `ChannelHandshake` 的 additive `version` 字段强制修改了 3 个既有集成测试文件的构造式（`tests/state_subscription_test.rs`、`tests/state_channel_contract_test.rs`、`tests/testsuite_e2e_test.rs`）：`ChannelHandshake { secret: X }` → `ChannelHandshake::new(X)`。**仅为编译所需的机械改写，未改任何断言或测试逻辑**。这是 additive 字段落在结构体字面量上的必然代价（design 未预见字面量构造点）。
- 6.1 的 `SessionRejection` 定义在 `sebas-domain`（非 `sebas-ipc`），因为 core 与 webui 都要用它；补 `Unknown` 时顺带在 `sebas-webui/src/api.rs` 增加了一个 HTTP 映射臂（502）。design 未点名该消费者。
- 6.1 有两处**记录的遗留缺口**（`PendingDisposition` / `PendingReason`），因 serde 的 `#[serde(other)]` 对普通 unit 枚举不适用而**未补齐**（需手写 `Deserialize`，超出本 change 的「补未知值路径」最小改动）。已在清单里具名 + 给理由 + 给触发条件，未假装完成。

**残余风险**：
- `SessionStreamFrame::Event` / `SessionEvent` 走的是「类型化解码失败」而非 `other` 变体：未知事件会让客户端重连重取快照并如实报告，但该帧本身不被接受。清单已记录；若将来需要「忽略未知事件继续读」，需给 `SessionEvent` 补 `Unknown`（会牵动 webui 的多处 `match`）。
- node-link 与浏览器边界按 design D8 / proposal Non-goals **未改结构**，其未知值路径是「核对记录」而非「本次加固」；它们的风险敞口与改动前相同。
- 7.4 的沙箱验证用的是 `fake-claude` 桩（合成应答），**不是**真实模型回合；真实凭据下的端到端由 operator 手跑的 `invoke smoke-real` 负责，本次未跑（无凭据、且明确不由 agent 自动运行）。
