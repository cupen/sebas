## 1. sebas-ipc 成为协议之家

- [ ] 1.1 在 `sebas-ipc` 增加 `protocol` 模块骨架与握手/framing 助手，依赖加入 `sebas-domain` / `sebas-channels`（仅叶子）；验证：`cargo build -p sebas-ipc` 通过，且 `cargo tree -p sebas-ipc` 不含任何角色实现（core / webui / router / im）与 sebas-node
- [ ] 1.2 加机械断言测试（放在根 crate 集成测试）：解析 `cargo tree -p sebas-ipc` 与 `sebas-ipc` 公开符号，断言不含角色实现、不含域表名；验证：新增测试通过，且临时给 `sebas-ipc` 加一条 `sebas-webui` 依赖时该测试失败（附一次失败演示）
- [ ] 1.3 crate 文档写明准入面（transport + wire 类型 + 握手/framing 助手）与「不得含角色实现」纪律；验证：`cargo doc -p sebas-ipc` 无警告，文档含准入清单

## 2. core channel 协议类型迁出根 crate

- [ ] 2.1 把 `CoreChannelRequest` / `CoreChannelResponse` / `SessionStreamFrame` / `StateStreamFrame` / `ChannelHandshake` / `Attachment` / `NodeLinkOp` / `NodeLinkOutcome` / `NodeView` 迁入 `sebas-ipc::protocol`，根 crate 原位 `pub use`；验证：`cargo test -p sebas` 全绿，且 `src/core_channel/protocol.rs` 不再定义这些类型
- [ ] 2.2 确认迁出后不再引用角色 crate 类型（依赖 change 1 已把中立契约类型搬到 `sebas-domain`）；验证：`cargo build -p sebas-ipc` 通过且 `grep -n "sebas_dispatch\|sebas_webui" sebas-ipc/src/` 无输出
- [ ] 2.3 复核 wire 形状零变化：对迁出前后的同一载荷做序列化比对；验证：逐字节一致（含 `cmd` / `frame` / `op` / `result` 标签与字段名）

## 3. core channel 握手版本协商

- [ ] 3.1 `ChannelHandshake` 增加带默认值的版本字段，服务端握手响应携带本端版本；验证：单测断言字段缺省时按版本 1 处理，且序列化含版本字段
- [ ] 3.2 版本不匹配 → 类型化拒绝，指名客户端版本与本端支持版本；验证：单测断言拒绝文本含两个版本号，且该连接上无请求被处理
- [ ] 3.3 认证次序不被削弱：peer-uid → secret → 版本 → 请求；验证：专项用例「错 secret + 不支持的版本」只报认证失败、不泄露版本支持（PostgreSQL 风格的 fail-closed 次序）
- [ ] 3.4 双向兼容用例：①新服务端 × 不发版本的旧客户端 → 按 1 服务；②旧服务端 × 带版本字段的新客户端 → 靠默认值解码成功；验证：两个用例各自通过，附握手原文

## 4. router 删除自备协议

- [ ] 4.1 `sebas-router/src/core_channel.rs` 改为复用 `sebas-ipc` 的握手、帧类型与请求构造，删掉本地 `StateStreamFrame` 子集（`:239`）、本地 `SnapshotResp`（`:223-233`）与内联 `json!` 请求；验证：`cargo test -p sebas-router` 全绿，且 `grep -n "StateStreamFrame\|SnapshotResp" sebas-router/src/` 只剩导入
- [ ] 4.2 secret 解析改为复用共享实现；验证：`sebas-router` 的既有 secret/鉴权用例不改而通过
- [ ] 4.3 用「core 侧改一个字段名 → router 编译失败」验证闸门生效；验证：临时改名后 `cargo build -p sebas-router` 失败，恢复后通过（附一次失败演示）
- [ ] 4.4 router 行为回归：hot-reload 与 state 订阅路径；验证：`cargo test -p sebas-router` 含 `process_e2e_test` / `contract_test` 全绿

## 5. golden fixture 契约闸门

- [ ] 5.1 core channel fixture：代表性载荷（请求、响应、快照帧、事件帧、拒绝）的序列化字节 + 字段名集合；验证：fixture 测试通过，且临时删一个字段时测试失败（附一次失败演示）
- [ ] 5.2 node link fixture 覆盖握手与版本协商消息；验证：fixture 测试通过，且既有 `PROTOCOL_VERSION` 逐字段比较与「指名双方版本」的拒绝行为不变
- [ ] 5.3 webui WS 与 HTTP fixture；验证：与既有 `ws_rpc_contract_test.rs` 不冲突、两套都通过，HTTP 侧形状与 `api_endpoints_test` 断言一致
- [ ] 5.4 复核「新增带默认值字段不触发 fixture 失败」；验证：临时加一个带默认值的字段，fixture 测试仍通过（证明闸门语义正确，不是全量卡死）

## 6. 前向兼容规则核对

- [ ] 6.1 逐条核对既有 wire 枚举是否都有未知值路径（对齐 `node-link` 的 `#[serde(other)] Unknown` 范例），缺的补齐；验证：产出一张「枚举 → 未知值路径」清单，清单无空缺
- [ ] 6.2 核对每条边界的新增字段是否都带 serde 默认值；验证：抽查三条边界的近期字段，均带默认值
- [ ] 6.3 把「新增字段须带默认值 / 枚举须留未知路径 / 删改属破坏性变更」写进 `AGENTS.md` 的协作约定；验证：文档含三条规则且与 `specs/ipc-protocol-home/spec.md` 一致

## 7. 全量回归与收口

- [ ] 7.1 记录本 change 唯一的有意 wire 变化（握手新增版本字段）并在 spec 与文档点名；验证：`proposal.md` 与 `design.md` 均有该条目，且无其它 wire 差异
- [ ] 7.2 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
- [ ] 7.3 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位并记录
- [ ] 7.4 跨角色联调复核：按 AGENTS.md 沙箱菜谱起 core（+webui）与独立 router，走一次会话创建与 provider 状态订阅；验证：`/api/summary` 的 `reachability.ok = true`、router 日志显示握手成功且订阅到状态帧
