## 1. 节点进程骨架与身份

- [x] 1.1 新增 `sebas-node` **独立 crate 与二进制**骨架（配置解析、上游主控地址、ready 握手；不走 `sebas` 的子命令面），验证：`cargo build -p sebas-node` 产出 `sebas-node`；`sebas-node --help` 描述配对/上游/node id；未配对时以 75 退出且 stderr 末行为 `startup-failure: <原因>`；`sebas --help` 的子命令树**未新增**任何条目
- [x] 1.2 node id 的确定与持久化（配对时由操作者确认、默认生成、落节点本地文件 0600），验证：单测覆盖「首次生成并落盘」「重装后同 id 重新配对沿用」「同一 id 二次上线被拒」
      —— 节点侧存取有单测（首次生成并落盘、显式采纳、复用、`0600`/目录 `0700`、非法输入不落盘）。
      —— 「重装后同 id 重新配对沿用」：进程级 e2e 用**同一个状态目录**重启节点（`node-id` 与 `credential` 都在里面）即沿用同一身份接入，不重新配对。
      —— 「同一 id 二次上线被拒」：主控侧单测 `same_id_online_twice_is_refused`；「换新状态目录 = 重装」的进程级用例（`a_reinstalled_node_makes_its_sessions_read_as_gone`）另行断言新身份下旧会话读作「节点侧已不存在」。
      —— 端到端确认（9.3）：真节点进程用一次性 token 配对待长期凭据，此后**丢弃 token 重启仍以同一 id 接入**。
- [x] 1.3 节点侧配置段（node id、上游主控、并发上限、保留期、provider 引用、默认工作目录），验证：配置单测覆盖缺省值与非法值报错，`cargo test -p sebas-node` 全绿
- [x] 1.4 节点机上的进程托管（systemd 单元或等价物，不走 watchdog 受管服务表），且与主控同机运行时不冲突，验证：沙箱同机起 `sebas`（core）与 `sebas-node` 两进程、两条链路互不干扰
      —— **就绪信号形态定为 `Type=simple`**：节点没有需要通知 init 的就绪握手（起来就拨号，拨不上按退避重试），因此不引入 `sd_notify` 依赖；永久性失败以 75 + stderr 末行 `startup-failure: <原因>` 退出，`journalctl -u sebas-node` 直接可读。
      —— 托管形态与可照抄的 systemd 单元落在 `docs/remote-execution-node.md` §7（含 `StartLimitIntervalSec/Burst` 以免永久故障无限空转、`StateDirectory` 持久化执行事实、只做出站连接故无需 inbound 端口与额外 capability）。
      —— **同机不冲突**：进程级 e2e 在同一台机器、同一个沙箱里同时起 `sebas core` 与 `sebas-node` 两个进程（不同状态目录、节点只出站拨号主控的回环监听），两条链路各自跑通且互不干扰 —— 9.3 的用例即是这条验证。
- [x] 1.5 依赖隔离：`sebas-node` 的依赖图内不得出现主控角色；共享协议类型抽成一个两侧都能依赖的小 crate，验证：`cargo tree -p sebas-node` 不含 `sebas-webui` / `sebas-feishu` / `sebas-im` / `sebas-router` / `sebas-dispatch`；`cargo build -p sebas-node` 在只构建该目标时通过
      —— 已核验：节点依赖图 69 行，主控 crate 与 openlark/rusqlite/axum/reqwest 均未出现；启动失败契约抽为 `sebas-startup` 叶子 crate（两侧共用，非复制）。**协议类型 crate** 待 2.x 定义协议时一并建立。
- [x] 1.6 打包面覆盖第二个产物（Dockerfile / CI / release / ansible），验证：本地或 CI 能分别产出 `sebas` 与 `sebas-node` 两个文件，且 CI 工作流包含该构建目标
      —— Dockerfile 补 `COPY sebas-startup sebas-node`（缺了会让 workspace 解析失败，属必须修复）、ci.yml 增 `--bin sebas-node`、release.yml `bin: sebas,sebas-node`；`--locked` 一致性已核验。ansible 暂无节点模板，留待 9.2/9.7。

## 2. 配对、链路与握手

- [x] 2.1 主控侧 join token 签发（一次性、带过期、可列出未用），验证：单测覆盖「消费后二次使用被拒」「过期被拒」「签发落库」
      —— `src/node_link/registry.rs`：原始 token 只签发时给出、文件内只留 SHA-256；消费立即落盘（否则主控崩溃后 token 可被重放）；`pending_join_tokens` 排除已消费/已过期。
- [x] 2.2 节点凭据交换与存储（token → 长期凭据，可吊销），验证：单测覆盖交换成功/失败、吊销后连接被拒且成因指名吊销
      —— 两侧都完成：主控侧签发/轮换/吊销/鉴权（「节点不存在」与「凭据不对」同码，不泄露存在性）；节点侧 `--join-token` 配对后凭据 0600 落盘，且**先落盘再声称成功**（落盘失败即配对失败）；吊销在服务端测试中断言码为 `credential_revoked` 且成因指名「吊销」。
- [x] 2.3 节点出站 websocket 客户端（拨号、指数退避重连、TLS 交由部署方、连接态上报），验证：集成测试用本地 ws 服务端覆盖拨号/断线重连/退避序列
      —— 真 ws 往返测试：拨号、配对成功、断线后自动重连（假主控接受两次连接）；退避 1s 起、翻倍、封顶 60s 有纯函数测试，且调参可注入使重试测试不必等待；`wss://` 如实拒绝（TLS 未实现，设计上由部署方终止）。
- [x] 2.4 握手协议版本 + 能力清单（agent kinds 及可达性、provider 清单、每执行体能否强制 mode），验证：双向单测覆盖清单序列化往返；版本不兼容时如实拒绝并同时报出两版本
      —— 清单内容已从空壳变成真话：节点配置 `[node.agents.<kind>]`（`command` 用于探测、`enforces_mode` 缺省 **false**——不假定做得到）与 `[node] providers`；`NodeConfig::manifest()` 组装清单，探测只做**存在性+可执行位**判定、**不执行任何命令**（握手路径上起子进程既慢又等于给远端一个任意执行入口）。
      —— 可达的判据是「命令在 PATH 上 **且** 本节点真能驱动它」：只满足前者仍记 `reachable:false`，成因写明「运行时尚未接入」；命令缺失则成因指名命令。清单随握手过链路，控制面侧 `NodeConnection::manifest()` 可读（工作台据此只提供可达项）。4 项单测 + 1 项 e2e。
      —— **载体与协商已完成**：契约类型落在 `sebas-node-link`（两侧共用，非复制），两侧线上交换已实现并测试（版本不兼容时报出两版本、非法 id 按**同一份**共享规则拒绝）；**清单内容仍为空**——填充需要节点侧 agent 配置（kind 列表 + 可达性探测 + mode 强制能力），随 group 3 的 agent 配置面落地。
- [x] 2.5 主控侧节点注册表（节点条目、在线态、最后上线时间、吊销状态），验证：单测覆盖上线/下线/吊销三态与「同名 id 冲突拒绝」
      —— 独立 JSON 文件（同项目注册表的取舍：自己有写者与生命周期），损坏文件隔离改名 + 空表启动；同 id 在线/已吊销再配对拒绝，离线再配对允许并轮换凭据（i2 重装接续）。
      —— 服务端共享**同一份**注册表句柄（`NodeLinkServer::registry()`）：并发连接不得各自 open 同一文件互相覆盖，锁只在注册表操作期间持有、绝不跨网络。
- [x] 2.6 把监听端点装进 core（设计 D13：由 core 托管）：配置项（监听地址、注册表路径）、随 core 启动/停止的生命周期、签发 join token 的管理入口，验证：沙箱内 core 起监听后 `sebas-node --join-token` 接入成功、节点在线态可在主控侧读到；core 退出后节点如实退避重试而不是静默退出
      —— 配置段 `[node_link]`（`enabled` 默认**关**、`listen` 默认仅回环、`registry_file` 缺省落 config 同目录、`bootstrap_token_ttl_secs` 默认 900；`SEBAS_NODE_LINK_LISTEN` 覆盖；`enabled` 时监听地址在**解析期**校验，不让坏地址拖到 ready 之后）；装配在 ready **之前** bind，失败即 75。
      —— **管理入口为 bootstrap token**：仅当注册表既无节点也无待用 token 时签发一次并记日志（沿用 webui 首启凭据「只显示一次」的既有形态），之后不再重复吐凭据；完整的签发/吊销管理入口（通道 RPC + 服务页）仍待补，见 2.7。
      —— 沙箱两进程实测（端口 0 由内核分配，全部隔离在一次性目录）：核心开放监听 → 节点用 bootstrap token 配对成功、凭据 0600 落盘、注册表 `dev-box online + last_seen`、待用 token 归 0 → 断开后 `offline` → 不带 token 复用凭据再次接入成功 → 已消费 token 再用被永久拒绝并以 75 退出（末行 `startup-failure: … join_token_consumed …`）。
- [x] 2.7 节点管理入口（签发 join token / 列出节点 / 吊销）经 core session channel 暴露，供 CLI 与 webui 服务页使用；必须使用 `NodeLinkServer::registry()` 那一份写者句柄，验证：集成测试覆盖三个操作与「越权/未知节点」的如实拒绝
      —— 通道新增 `NodeLink { op }` 请求与 `NodeLinkOutcome` 应答（`IssueJoinToken` / `ListNodes` / `RevokeNode`，附 `Disabled` 与 `Failed` 两种如实否定）；core 侧句柄在 `arm_core_channel` **之前**打开并同时交给监听与通道，两处共享同一份写者（`NodeLinkServer::bind_with` 明确接受已有句柄，杜绝另开实例）。
      —— CLI `sebas node-link token|list|revoke`（token 打 stdout 便于 `$(...)`，指引打 stderr；未知节点吊销 → 非零退出且明确「未做任何改动」）。
      —— 单测 2 项（未启用如实回 `Disabled`；签发→列表→吊销→`found=false` 全环）+ 三进程沙箱实测：CLI 签发 token → 节点接入 → `list` 显示 online → 断开后 offline → `revoke` → `list` 显示 revoked → 被吊销节点再接入**永久拒绝并以 75 退出**（末行 `startup-failure: … credential_revoked：节点 dev-box 的凭据已被吊销`）→ 吊销不存在的节点非零退出。


## 3. 会话身份、放置与协议骨架

- [x] 3.1 控制面发行会话 id（项目命名空间）并随项目维度建索引，验证：单测覆盖「同项目内不会重复发行」「不同项目可同名会话」「id 由控制面生成而非节点」
      —— `RemoteSessionId`：形状 `<项目命名空间>:<8字节hex>`，无项目用 `(no-project)`；1000 次发行断言唯一；**同项目内同名不撞**（完整 id 不同、仅后缀相同）；节点从不自造 id（协议里 id 只由控制面在 `Spawn` 里给出，节点以它为主键建本地日志）。
- [x] 3.2 项目条目扩展为 (节点, 路径)：注册数据形状、迁移旧数据为隐式本机节点，验证：迁移单测 + `openspec/specs/agent-workbench` 场景对应测试（同路径两节点 = 两条目；本机隐式注册行为不变）
      —— `ProjectEntry.node_id`（serde default = `local`）：**迁移即回填**，无迁移脚本；`project_id_for_on` 在**本机保持历史公式**（既有 id 不变，否则既有项目成孤儿），远端把节点名拌进哈希 → 同路径两节点 = 两个 id、两条目；`add_on` 对远端**不做任何本地文件系统操作**（可用性由节点在 spawn 时判定，见 3.3）；重复判定按 `(节点, 路径)`。6 项新测试。
- [x] 3.3 路径可用性判定移到节点侧：节点在 spawn 前校验并返回 typed rejection（含路径与成因），验证：集成测试覆盖「路径不存在」「路径不是目录」「可用路径正常建会话」
      —— `SessionHost::spawn` 在**节点本地**判定（`is_dir`），拒绝码 `unusable_project_dir` 且成因**指名路径**；双向 e2e 实测「不存在路径被拒 + 成因含路径名」「存在目录正常建会话」。
- [x] 3.4 放置实现：项目决定节点、无项目会话落到配置的默认执行节点、节点离线时如实失败且不建占位会话，验证：单测覆盖三条路径；离线失败返回指名节点的成因
      —— `resolve()` 只做决策（可在无节点在线时独立测试与呈现）：项目决定节点且默认节点不参与；无项目 → 默认执行节点，未配置则如实失败 `NoProjectAndNoDefaultNode`；`place_and_spawn()` 节点离线 → `NodeOffline{node_id}`，**且 e2e 断言真节点上零会话（不建占位、不排队）**；节点拒绝则带节点名与拒绝码返回。
- [x] 3.5 会话级操作过链路：prompt / cancel / close / set_model 的请求-应答与 typed rejection 透传，验证：本地 ws 双向集成测试覆盖成功与拒绝，且不泄漏驱动内部词表（协议形状断言）
      —— 协议只有会话级操作（`SessionOp`）与条目（`LogEntry{kind:String}`），无 AcpCommand/AcpEvent 泄漏；拒绝是**正常应答**（带码，决定重试与否），不是错误。双向 e2e 覆盖 Spawn/Prompt/LogFrom/Snapshot/Close 成功路径 + `session_closed`/`unknown_session`/`unusable_project_dir`/`unsupported_agent_kind` 四类可判别拒绝。

## 4. turn 流、日志与持久化

- [x] 4.1 节点本地会话日志（按会话 append-only、`seq` 单调、`epoch` 随重置递增），验证：单测覆盖追加顺序、重启读回、重置后 epoch 递增
      —— `sebas-node/src/log.rs`：`.log.jsonl` 只追加 + `.meta.json` 原子重写；**重启读回序列继续**；重置递增纪元并从零开始；崩溃留下的半截行只丢那一行（不整段不可读）。
- [x] 4.2 节流合并上行（默认窗口 100–250ms、可配）与缓冲上限，验证：单测断言窗口内多事件合并为一批；超限时产出「已合并/可回拉」的明示事件而非静默丢弃
      —— 默认窗口 150ms、单批上限 256 条，两者可注入；窗口内多事件合并为一批；超限**压缩并打 `coalesced_overflow` 标记**，且被压掉的条目仍在日志里（可回拉）——测试断言两件事都成立。
- [x] 4.3 按 seq 范围回拉精确序列的接口，验证：集成测试对同一段 turns 断言「合并流 + 回拉 = 原始精确序列」
      —— `LogFrom` + `SessionLog::since`；单测断言「合并后的批 + 回拉 = 原始精确序列」，双向 e2e 亦按 seq 回拉到含 `echo: hello` 的精确内容。
- [x] 4.4 保留期回收 + 回收水位线上报（主控把该段标记为 unavailable-at-node），验证：单测覆盖「按保留期回收」「水位线上报」「主控视图缺口可回答而非 pending」
      —— `SessionLog::reclaim_older_than(cutoff)`：**只回收连续前缀**，年龄未知（`at_unix == 0`）的条目保守不动，绝不从中间挖洞（挖洞＝制造一个永远补不上的缺口）；水位线持久化在 `.meta.json`，跨重启仍读得到。
      —— **策略是节点自主的**（r2：`log_retention_days` 配在节点侧，不是主控下令的清理）：`SessionHost::sweep_retention` 按保留期扫全部会话（含重启后挂回来的孤立日志），把推进的水位线作为 `Reclaimed` 事件放进 outbox。**离线也照扫**——水位线就在 outbox 里等重连上报，否则断链期间的回收会让控制面永远等着一个 pending 的缺口。
      —— 链路层挂小时级巡检（`RETENTION_SWEEP_INTERVAL`，`interval` 首次 tick 立即触发，故启动也扫一遍）；`LinkTuning::retention_sweep` 可注入，测试不必等一小时。巡检任务由 `run` 持有，返回时一并收走，不留孤儿。
      —— 主控侧 `RemoteSession::note_reclaimed`：把落在水位线内的缺口从 pending 迁到**永久缺损**，部分覆盖则切段——缺口因此是「可回答」的，而不是永远悬着。
      —— 测试：log 层 5 条（前缀/不挖洞/未知年龄/跨重启/时间戳）+ 宿主层 1 条（40 天前的日志被回收、水位线进快照、`Reclaimed` 上报、重扫幂等无噪音）+ 链路层 1 条（真链路跑起来，假主控在 Event 帧里收到水位线）+ 主控侧 2 条（缺口转永久缺损、部分回收切段）。
- [x] 4.5 磁盘上限触顶时如实拒绝新会话且不丢既有历史，验证：集成测试构造触顶，断言新会话被拒且既有日志完整
      —— 触顶回 `storage_exhausted`（**瞬时**码：清理/扩容后可恢复），并断言既有会话的日志一条不少。
- [x] 4.6 节点侧并发上限与诚实拒绝，验证：单测覆盖达到上限后的 typed rejection（不排队、不挤占）
      —— `over_capacity`（瞬时码）；只数**活**会话，关闭即释放额度，且关闭的会话日志仍可查。

## 5. 对账（快照 + 增量）

- [x] 5.1 状态快照（幂等）：节点上报当前态、主控整体替换该会话投影，验证：单测断言重复应用同一快照结果不变
      —— 协议与驱动层：`Snapshot` 操作 + `RemoteSession::note_snapshot`（整体替换相位/纪元/水位线/末序号，重复应用不变）。
      —— **已接入 core 的会话投影**（`src/node_link/projection.rs`）：远端会话以确定性行键（`node\0<节点>\0<会话>`）出现在 `Snapshot` 响应与 `Subscribe` 流里，行上带 `remote{node_id,node_status,node_cause,desired_mode,effective_mode,parked_approvals}`（`sebas-dispatch::RemoteSessionView`，`serde(default)` 兼容旧报文）。本机会话的键没有该前缀，两条路径不会互相覆盖。
      —— 幂等有单测守着：重复应用同一批 → 行逐字不变、条目数不变（`batches_become_transcript_entries_with_the_nodes_timestamps`）。
      —— 单测还钉住三件事：**在等人批的会话不得显示成在跑**（`parked_approvals > 0` → `dormant`）；链路断开是 `offline` 而**不是**终止（`is_alive()` 仍为真、成因如实带出）；节点报的退出才是 `terminated` 且成因来自节点。
- [x] 5.2 增量拉取游标：主控持游标、节点从游标处续传，验证：集成测试覆盖「断连期间产生 12 个 turn → 重连补齐且不重不漏」
      —— `RemoteSession{cursor}` + `reconcile()`（从游标 +1 回拉）；**幂等**：重复应用同一段 → `Overlapped{added:0}` 且视图不变（双向 e2e 与单测各验一次）。
- [x] 5.3 重连与主控重启走同一段对账代码，验证：两条路径（链路抖动 / 主控进程重启）走同一函数，集成测试各跑一遍断言结果一致
      —— 对账代码收敛为 `RemoteSession::reconcile`（幂等，从游标 +1 回拉）。
      —— **已接到 core 的重连路径**：`RemoteProjection::observe_node` 是唯一的重建入口，由 `ProjectionObserver::connected` 调用——链路抖动重连与主控进程重启（视图为空）走的是同一条路：`ListSessions`（认领身份，**不重建**）→ 逐会话 `reconcile` 回拉 → 起事件消费者。**先订阅再拉快照**（反序会漏掉两者之间的批）；节点事件与本地事件在 core 通道的同一个出口合流。
      —— 断开只标 `NodeOffline`（`RemoteFleet::on_node_disconnected`，**不终止**）；节点回来时只把「只是离线」的会话恢复为在跟踪，**不动已终止的**（`set_node_live`）。
      —— 说明：链路抖动路径由既有 e2e（杀节点/断链路/杀主控三类）覆盖 `RemoteFleet` 层；投影层的新增单测覆盖行组装与事件归并。进程级「杀主控重启后对账补齐」的端到端用例归 9.3。
- [x] 5.4 epoch 变更处置：主控把时间线标为不连续而非追加，验证：单测 + 集成测试（重置节点日志后重连）
      —— `RemoteSession::segments`：纪元变化**开新段**而不是把两条时间线接成一条；同纪元重复上报幂等；**纪元未知（0）时首次学到＝校正，不是断裂**（turn 批原先不带纪元，会把两种情形混为一谈——已修协议：`TurnBatch` 现携带 `epoch`，`0` 表示未知以兼容旧发送方）。
- [x] 5.5 悬空审批纳入对账（重报仍 outstanding 的、不复活已决议的），验证：集成测试覆盖「主控停机期间 park 2 条 → 返回后全部可见且可决议」「已决议者不重现」
      —— `RemoteSession::reconcile_approvals` **以节点为准整体替换**本地悬空集合（悬空的事实由节点持有）；事件流增量维护且按 `request_id` 去重（重连重放不产生两条）。测试：本地陈旧地以为悬着 2 条 → 对账后只剩节点承认的那 1 条 → 再对账为 0（已决议者不重现）；决议可路由回并按节点回报区分「生效 / 被丢弃」。
- [x] 5.6 会话寿命绑定节点：主控重启不终止、链路断开不终止、节点重启终止且如实报告、对账不重建，验证：三类进程级测试（杀主控 / 断链路 / 杀节点）断言会话状态
      —— `RemoteFleet` 把三种情形分成**三个不同结论**：`Live` / `NodeOffline`（链路断，**不终止**）/ `Terminated`（节点重启，成因来自节点）。e2e 三类都测：① 主控视图整个重建 → `adopt_from_node` 认领回身份、仍 `Live`、日志照样拉回、**节点上仍只有那一个会话**（不重建）；② 停节点 → 链路断开只标离线；同一状态目录重启 → 节点上报 `terminated`，控制面如实记为终止；③ 换新状态目录（重装）→ 「节点侧已不存在」。
      —— 配套修了两处真问题：**节点重启后宿主挂回孤立日志**（此前磁盘上还在的日志在协议上再也拉不到——执行事实凭空消失），且扫描目标是 `*.log.jsonl` 而不是 `*.meta.json`（meta 只在回收/重置时才写，按它扫描等于一个都挂不回来）；服务端把「标记离线」排在「移出 live 表」**之前**，否则重连瞬间会撞 `NodeIdConflict` 而被永久拒绝。

## 6. mode 与审批

- [x] 6.1 session mode 领域模型（`ask` / `edit` / `allow` / `auto` + desired/effective 双字段），验证：单测覆盖序列化往返与缺省非 auto
      —— 协议里是类型（`SessionMode`，缺省 `ask`，**`auto` 永远不是缺省**）；`SessionSummary` 同时给出 `mode`（实际生效）与 `desired_mode`（期望），差异即「执行体强制不了」的如实呈现；不认识的模式**拒绝而不降级**（`unsupported_mode`）。
- [x] 6.2 mode 门：`auto` 之下不产生审批请求；其余模式按门产生，验证：单测 + 集成测试（auto 会话跑 gated 工具无请求；ask 会话产生请求）
      —— 门控粒度是**动作类别**（edit/execute/other）：`ask` 全问、`edit` 只放编辑、`allow` 全放但留审计、`auto` 不门控；单测覆盖四档与三类组合，e2e 覆盖「ask 停驻」「auto 无请求」。
- [x] 6.3 执行体 mode 落地：原生内核走 policy engine 强制；ACP/Claude 侧尽力映射并把实际生效值回报（无法强制则如实上报），验证：单测覆盖「可强制 → effective = desired」「不可强制 → effective 标为未强制且差异可见」
      —— 机制：`ExecutionBody::mode()` 回报自己真正执行的模式；新增 `enforces_mode()` / `resolve_gate()` / `drain()` 三个**带默认实现**的钩子，执行体不必全实现。
      —— **真实执行体已落地**（`sebas-node/src/body.rs` 的 `AcpBody`）：真子进程 + ACP 握手，同步/异步用一个专职工作线程上的 current-thread runtime 对接；门控请求变成 `GateRequest.resume_token`，审批决定经 `answer_approval` 回灌给停住的那一轮（投不到就如实回 `applied:false`）。
      —— 验证（对着仓库自带的 `fake-claude-cli` 桩，**真子进程**）：完整一轮、以及 gate 的「停住 → 放行 → 工具结果」全环；`enforces_mode=true` → effective=`ask`；未声明 `enforces_mode` → `mode: None`（**不冒充**）而 `desired_mode: ask` 仍在 —— 差异因此是可见的；`cancel`/`set_model`/`close` 的诚实性各有用例（空闲取消回 false、模型切不动回带成因的 typed error）。测试：`sebas-node/tests/acp_body_e2e_test.rs`（6 passed）+ lib 111 passed。
      —— **未验证**：真实 Claude Code / 真 agent CLI（本机没装、也没有真凭据）——桩说的是同一套协议，但它不是真 agent。**原生内核执行体不做**：把内核搬上节点会把主控角色拖进依赖图、破坏 D0，属 9.7 的开放问题。
- [x] 6.4 远程审批上行走廊（请求上行、决定下行、按 request_id 关联），验证：通道全环集成测试（帧到达 + 决定路由回）+ 未知 id typed rejection
      —— `ApprovalRequested` 上行 / `ApprovalAnswer` 下行 / `ParkedApprovals` 对账；`request_id` 形如 `<会话>:req-<n>`；未知或已决议过的 id → `unknown_approval_request`（永久）。e2e 在**真节点**上跑完整环：请求到达 → 决议 → 节点日志留审计 → 重复决议被拒。
- [x] 6.5 主控不可达时无限期 park（不超时、不降级放行），验证：集成测试断开主控并在超长等待后断言请求仍 parked、工具未执行
      —— 节点侧没有任何计时器参与裁决；测试把时间窗推后**一小时**断言：无 `GateResolved`、请求仍停驻、日志里既无 allow 也无 deny 痕迹。工具在获准前不执行（断言无 `output` 条目）。
- [x] 6.6 节点侧不存在任何本地裁决路径（安全性质），验证：接口面审计（node 进程无审批决定入口）+ 集成测试断言断开主控时无任何本地放行
      —— 唯一能改变停驻状态的入口是 `SessionHost::answer_approval`，而它只被链路请求调用；节点 CLI 没有、配置没有、超时也没有。上面的「推后一小时」测试即是这条性质的断言。
- [x] 6.7 迟到/无效决定丢弃 + `auto` 选择留审计痕迹，验证：单测覆盖「会话已终止后到达的决定被丢弃并给出提示」「mode=auto 变更写入审计事件」
      —— 会话已关闭时 `ApprovalApplied{applied:false}` + 日志写「丢弃迟到决定 {id}」（控制面侧同时记入 `discarded_decisions`，不让「点过允许却没生效」无声消失）；未知 id 则是 `unknown_approval_request`。`auto`：自动放行事件带 `source=mode:auto`，且**开启 auto 本身**写入审计条目；e2e 断言 `LogFrom` 能看到它。

## 7. provider 与材料

- [x] 7.1 节点 provider 清单上报与 desired/effective 应用（不可用时如实回报实际值），验证：单测覆盖「可应用」「不可应用 → effective 与 desired 不同且可见」
      —— **清单上报**：`[node] providers` 与 `[node.provider_profiles.*]` 的名字并集如实进清单；`upstream = control-plane-router` 时 provider 列表**为空**（节点零凭据，不是「暂时没填」）。
      —— **provider profile 已建模并落地**（`sebas-node/src/config.rs` + `body.rs`）：`[node.provider_profiles.<name>]`（protocol / base_url / api_key_env）+ `default_provider`；校验期就拒绝 `base_url` 非 http(s)、指向不存在的 default。
      —— **desired/effective 双轨**：协议里 `Spawn.provider` 进、`Spawned.provider`/`provider_cause` 与 `SessionSummary{provider, desired_provider, provider_cause}` 出；`RemoteSessionView` 同名列随会话行下发（8.x 的呈现层据此显示「没生效」而不是把期望值当结果）。单测覆盖「可应用」「未知 profile（点名）」「缺凭据环境变量（点名）」「default_provider 兜底（desired 保持 None + 成因）」。
      —— **未验证/未做**：控制面目前不下发期望 provider（没有任何上层会去选 profile），所以真正端到端的「desired ≠ effective 且操作者可见」只到协议与投影层；前端也还没渲染 provider 行。
- [x] 7.2 可选 `upstream = control-plane-router`：节点零凭据、模型流量经主控 router，验证：沙箱内做一次模型请求走通，并断言节点侧不落任何 provider 凭据
      —— **两端都接上了**：主控在握手里告知 router 端点（`HelloAck.router_url`/`router_token`；`run.rs` 从内置 router 的**真实**监听地址与它的下游 token 填充，没启用 router 时是 `None`）；节点在 spawn 时把 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`（或 openai 侧对应项）注入子进程，**不落盘**任何 provider 凭据。
      —— **没有地址就如实拒绝**：该模式下拿不到地址时 spawn 回 `ProviderUnavailable` 并点名缺什么，绝不猜地址（有 e2e 用例真跑这个拒绝路径）。
      —— **未验证**：注入的 env 真正到达子进程、以及一次模型请求真的穿过运行中的 router——本环境没有能发起模型调用的 agent（桩不发模型请求）。另一条要如实说明的边界：`sebas-acp` 的 `extra_env` 只能**增加**变量，节点无法清掉自己进程里已有的 provider 变量，所以「节点零凭据」依赖运维把节点环境弄干净。
- [x] 7.3 操作者级材料拉取（spawn 时拉取、落点按执行体、会话创建钉版本），验证：单测覆盖版本钉住；集成测试断言「主控更新材料后旧会话仍用旧版、新会话用新版」
      —— 节点侧 `MaterialStore`（版本目录隔离 + 先写 staging 再 rename + 同版本幂等）与 `place_for`（**落点是动作**：把材料真正复制进执行体读的目录，只算路径会留个空目录 = 看起来配好其实没生效）；控制面侧 `MaterialStore` 应答来向请求（空仓**如实拒绝**而不是发空包；点名旧版本也拒绝，不悄悄给新版）。
      —— 钉版本：`set_pending_materials` **取走语义**（不会漏到下一个会话），`Spawned.materials_version` / `SessionSummary.materials_version` 双处回报，日志留 `materials_pinned` 条目（事后能回答「当时用的是哪一版」）。e2e：v1 落在 `<state>/materials/v1/echo/…`、旧会话仍报 v1、新会话拿到 v2、v1 内容不被覆盖。
- [x] 7.4 变更通知只带版本/失效信号（不带内容），验证：协议形状断言 + 集成测试（通知后由节点发起拉取）
      —— `MaterialsChanged { version }` 的**形状断言**：JSON 里不得出现 `files`/`content`（防止日后有人顺手把内容塞进广播）。e2e 断言**惰性拉取**：控制面换版本本身不产生任何拉取动作（`materials/v2` 不存在），只有下一个会话创建时才拉——这正是「通知只是信号」。
- [x] 7.5 落点表与不支持时的如实拒绝（原生内核 / Claude Code 至少两种落点），验证：单测覆盖各落点路径与「无落点的执行体被如实拒绝而非静默忽略」
      —— 落点表：`echo` / `native` 各有专属目录（同一版本下互不共用），落地动作幂等且不自嵌套；**ACP 类执行体（claude 等）如实拒绝**（`NoPlacement`，成因写明「落点约定尚未接入」）——绝不静默忽略，那会让操作者以为材料生效了。

## 8. 工作台与 CLI 呈现

- [x] 8.1 项目注册带节点维度（含本机隐式节点），验证：前端单测覆盖「注册带节点」「同路径两节点两条目」「本机隐式注册」
      —— `ProjectEntry.node_id`（`serde(default)` → `local`）；注册远端项目**由节点判定路径**：先确认节点已知且在线，再让节点 `CheckPath`，做不到就如实回「校验未完成」而**不是**「路径不对」（两种失败文案分开，有单测钉住）；`reorder` 改为按条目（含节点）定位，同路径两节点不再被合并成一条。
      —— 迁移（9.1）：旧注册表没有 `node_id` 的条目经 serde 默认回填为 `local`，无需迁移脚本（单测 `legacy_entry_without_node_id_is_read_back_as_local`）。
- [x] 8.2 节点离线呈现 + composer 阻止提交 + 免刷新恢复，验证：沙箱内 kill node 后断言项目标离线并阻止提交；重启 node 后免刷新恢复
      —— `GET /api/nodes`（唯一真源：core 的节点注册表 + 隐式的本机条目，永远在线并标 `local`）；项目行/会话行标注节点与成因，节点不在线时 `+` 与 composer 直接禁用（不是提交才失败）；前端 10s 轮询 `/api/nodes`，因此**免刷新**恢复。
      —— 实跑（真 core + webui + sebas-node，进程级）：节点在线 → 杀掉后 `/api/nodes` 转 offline、项目 `accessible:false`、离线时建会话得到点名节点的诚实拒绝 → 用原凭据重启节点后转回 online，全程没动 webui/core。
- [x] 8.3 desired vs effective mode 呈现 + `auto` 会话可区分，验证：前端单测覆盖「不可强制时显示差异并说明」「auto 会话有明确标记」
      —— 会话行/详情带 `remote.desired_mode` 与 `remote.effective_mode`；前端在两者不同时**同时显示两个值**并写明「执行体无法强制」；`auto` 标 `ungated`；`parked_approvals > 0` 时投影为 `waiting`（非终态才改写，已 Done/Failed 的不因历史审批看起来还在等人）。
      —— 单测覆盖「不一致 → 两个值 + 说明」「一致 → 不提示」「auto → 明确标记」。
- [x] 8.4 悬空审批呈现（等待 ≠ 运行中），验证：前端单测覆盖「parked 会话呈现为等待」「返回后可达其请求」
      —— 「等待 ≠ 运行中」：投影在有悬空审批时把行降为 `dormant`（**不显示在跑**），前端呈现为 `waiting` + 独立的「Waiting on you」分组，等待态没有「运行中」的脉冲动画。
      —— 「请求可达」：节点的 `ApprovalRequested` 立刻上行到与本地审批**同一个出口**（`RemoteProjection::notice_feed` 并入 core 的审批帧流），重连对账时把仍悬空的请求**主动重播**一遍（操作者回来就能看到谁在等）；决定经 `ApprovalAnswer` 按 `request_id` 路由回节点（`applied:false` = 决定迟到、会话已结束，与链路失败分开表达）。
      —— 进程级 e2e 断言：远端会话收到受门控动作后行转 `waiting`、`parked_approvals > 0`。
- [x] 8.5 会话/项目标注所属节点与离线成因，验证：前端单测 + 沙箱目视核对
      —— 会话行/项目行/项目头部都有节点 chip；不可达时给出**成因**（不是笼统的"离线"）；`node_status` 区分 `online`/`offline`/`terminated`。
      —— 前端单测覆盖标注与成因；沙箱实跑核对到 `/api/nodes` 与 `/api/sessions` 的 JSON 层面（**没做**浏览器目视：本环境的 webui 只能嵌占位页，见 9.3/9.4 的说明）。

## 9. 迁移、文档与收尾

- [x] 9.1 旧项目注册迁移为隐式本机节点，且无节点注册时主控行为与今日完全一致，验证：迁移单测 + 全量 `cargo test` 无回归
      —— 迁移靠 `#[serde(default = "default_node_id")]`：旧注册表条目自动成为本机项目，不需要迁移脚本；单测 `legacy_entry_without_node_id_is_read_back_as_local` 钉住。
      —— 「无节点注册时与今日一致」：`[node_link] enabled = false` 时通道拿到 `None` 投影、`NodeLink` 管理操作如实回 `Disabled`；`spawn_with(..., node)` 的 `None`/`local` 走原路径；沙箱默认配置**不加** `[node_link]` 段，既有旅程一字未改地跑通。
      —— 全量 `cargo test --workspace`：除**一处环境相关**失败外全绿——`tests/config_test.rs::validate_runtime_rejects_missing_binary` 在本沙箱里先撞上「`/home/cupen/.config/sebas` 不可写」（本会话的文件沙箱只允许工作区内写入），于是报的是目录不可写而不是二进制缺失；在有可写 HOME 的正常环境里不成立。
- [x] 9.2 文档：配对步骤、网络/TLS 要求、主控丢失的后果（ask 悬空会话不可恢复）、项目树须由运维准备，验证：`docs/` 与 README 相应章节落地并在沙箱按文档走通一遍
      —— `docs/remote-execution-node.md`（配对 / 网络与 TLS / 主控丢失的后果 / 项目树前提）+ `README.md` 的「远程执行节点」段落。
      —— **按文档在隔离沙箱里实跑过**：监听起来 + bootstrap token 只打印一次；`sebas node-link -c <配置> token`（`-c` 必须在子命令**之前**）；节点 `--join-token` 配对后 `credential`/`node-id` 皆 0600（且不改变既有目录权限）；不带 token 重启复用凭据；`revoke` 后重连以 75 退出、成因 `credential_revoked`；同 id 重新配对以 75 退出、成因 `node_id_conflict`；`wss://` 与未配对都以 75 退出并说明原因；`enabled = false` 时管理命令以 1 退出说「节点链路未启用」。
      —— 文档里**没有**声称验证过的：真实 TLS 终止反代拓扑（部署相关）。
- [x] 9.3 进程级 e2e 用例（core + node 双进程）：建会话 → 跑 turn → 杀主控重启 → 对账补齐 → 决议悬空审批，验证：`invoke testsuite-e2e` 新用例通过
      —— `tests/testsuite_e2e_test.rs::remote_node_pairs_survives_node_and_core_restarts`（`#[ignore]`，同套件约定）：**core 与 sebas-node 是两个真进程**，一次性 bootstrap token 从 core 日志里读出来配对 → 远端项目由节点判定路径 → 在节点上建会话并跑通 echo 一轮（经 webui HTTP 看到 `echo: hello`）→ **杀主控重启 → 对账把这一轮的转写补回来** → 发受门控动作使会话转 `waiting` 且 `parked_approvals > 0` → 杀节点（如实离线）→ 用原凭据重启节点（免刷新恢复，且那个会话还在）。
      —— 这条测试抓到并修掉**三个真问题**（都是单侧测试测不出来的）：
        ① **一次性 join token 被复用**：配对后 `LinkClient` 仍拿 token 重连，主控回 `join_token_consumed`（永久拒绝），于是**一次链路抖动就把节点永久打死**；现在配对成功即作废内存里的 token，改用落盘的长期凭据（回归单测 `a_spent_join_token_is_retired_in_favour_of_the_credential`）。
        ② **`announce_parked` 死锁**：`for id in self.fleet.lock().await.session_ids()` 里的 `MutexGuard` 临时量活到整个循环结束，而循环体又要拿同一把非重入锁 → 永久挂起（空集合时不显形，所以只在真有会话时咬人）。
        ③ **快照冒充"我已有全部条目"**：重建出来的空视图先应用快照就把游标推到了节点日志末尾，随后的增量回拉从末尾开始、一条也拉不回来——**主控重启后会话转写凭空消失**。现在拆成 `note_state`（只校正纪元/相位/材料/水位线，不动游标）与 `note_snapshot`（确认持有后才推进游标），并有单测 `a_snapshot_corrects_state_without_claiming_we_hold_the_entries`。
      —— 另外把沙箱补严了一条：`SEBAS_PROJECTS_PATH` 之前没被覆盖，注册项目会去写**操作者真实的** `~/.sebas/projects.json`（在只读文件系统上才暴露成"写临时文件失败"）。
- [x] 9.4 验收旅程（webui + node）：项目带节点注册、离线呈现、mode 差异、悬空审批，验证：`invoke testsuite-acceptance` 新旅程通过
      —— `tests/testsuite_acceptance_test.rs::remote_node_workbench_journey`（`#[ignore]`，同套件约定）：起 core + webui + 真 sebas-node，配对后经 **HTTP** 断言节点在线且本机为隐式条目、远端项目由节点判定注册、会话行带 `node_id` 且 `project_id` 与 `project_id_for_on(node, path)` 一致、受门控动作后呈现 `waiting`（**绝不呈现 working**）、杀节点后 offline 且成因非空 + `accessible:false` + 建会话得到点名节点的诚实拒绝、重启节点后恢复在线。**实跑：1 passed**。
      —— **未断言**（诚实列出）：desired/effective 的**差异**（创建路径不带 mode，差异要等对账学到节点摘要）、以及 `/ws` 评审卡上的"悬空请求可点"（见下）。
      —— 已知未闭环：webui 的 `/ws` 在本沙箱的进程级旅程里**一帧都没下发**（HTTP 面全通），评审卡因此没法验证——这是 core 通道/WS 侧的呈现面问题，不属 sebas-webui。
- [x] 9.5 与已归档的 `2026-09-10-unify-permission-approval-vocabulary` 核对：它改 `permission-flow` 的「Three decision outcomes」，本 change 改「Fail-closed on missing responder」并新增两条，验证：逐条比对确认无同一 requirement 的双写，记录结论
      —— 结论与逐条比对记在 `openspec/changes/add-remote-execution-node/notes/cross-change-check.md`：两边 requirement 名集**不相交**（归档改「Three decision outcomes」；本 change 改「Fail-closed on missing responder」+ 新增两条），**无同一 requirement 的双写**；且本 change 的 MODIFIED 基线取自归档后的 main 规格（逐字一致），不是归档 delta。
- [x] 9.6 glossary 修订：「单一二进制，子命令决定人格」改为「主控是单一二进制；执行节点 `sebas-node` 是第二个可分发产物」，进程角色表增列节点，验证：`grep -n "单一二进制" openspec/glossary.md` 不再作为全局断言；节点词条含「独立二进制、不含主控角色」
      —— 标题改为「进程角色（主控是单一二进制，执行节点是第二个可分发产物）」，并加了**范围**说明：`单一二进制` 只描述主控，不描述整个系统；进程角色表增列 `sebas-node`（独立二进制、不含 core/webui/router/im、反向拨号、持有执行事实、永不自我放行、自带 `[node]` 配置），易混对照里补了「`sebas`（主控）vs `sebas-node`」一行。
- [x] 9.7 bd 立项遗留项（升级编排、材料落点表形态、首次配对 UX、多主控、跨节点等价判定、节点是否承载原生内核），验证：`bd list` 可见对应 issue 且本 change 归档清单引用之
      —— `sebas-jdu`（升级编排）、`sebas-5x1`（材料落点表形态）、`sebas-wmq`（首次配对 UX）、`sebas-c2v`（多主控）、`sebas-isf`（跨节点等价判定）、`sebas-ilf`（节点是否承载原生内核），皆带 `add-remote-execution-node` 标签，正文写明「开放问题 / 为何推迟 / 什么能解决」。
