//! 原生 sebas-agent 执行体桥（make-feishu-optional-webui-primary，design D2/D3）。
//!
//! 把 router 的 `agent-*` 会话执行路由到原生内核（`sebas_agent` 的
//! `SessionManager`），而不是 acp 桥。飞书侧是唯一调用方：`DispatchHandle`
//! 在 `on_text`/`PassThrough`/`Btw` 看到 `agent-*` key 时经
//! [`sebas_dispatch::native_bridge::NativeSessionBridge`] 转发到这里。
//!
//! 会话状态登记进 router（`Mapping` + `turn_log` + `SessionEvent` + 权限广播），
//! 所以 webui 与 core session channel 能像看 acp 会话一样看到原生会话——
//! 权限请求走 `AcpEvent::PermissionRequest` 形状被 `InProcessBackend`
//! 中继到 webui 审查卡（fail-closed：无答即拒）。

use sebas_agent::session::{AgentEvent, SessionManager};
use sebas_channels::ChannelKey;
use sebas_dispatch::native_bridge::{NativeApprovalDecision, NativeSessionBridge};
use sebas_dispatch::{DispatchHandle, TurnEntry};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 一个原生会话的存活状态：内核句柄（供续聊与权限回填）。
#[derive(Clone)]
struct NativeSession {
    handle: sebas_agent::session::SessionHandle,
}

/// 桥本体（Arc 可克隆、唯一实例由 `run` 装配）。
/// 内部用 `std::sync::Mutex`：`answer_permission` 是同步 trait 方法，
/// 不能 `.await`，所以锁必须是非阻塞的。
pub struct DispatchNativeBridge {
    manager: Arc<SessionManager>,
    router: DispatchHandle,
    /// 编码后的 ChannelKey → 会话。
    sessions: Arc<Mutex<HashMap<String, NativeSession>>>,
    /// 待决权限请求：request_id → 内核 session_id（供回填）。
    pending: Arc<Mutex<HashMap<String, String>>>,
    /// 新会话的默认执行体：`true` = feishu 新会话走原生内核；`false` =
    /// 走 acp 桥（现状）。既有原生会话不受此影响（按 sessions map 判定）。
    default_native: bool,
    /// （add-local-usage-statistics 3.2）native 直连回合的本地落账 sink：
    /// pump 在 `SessionSummary` 帧结算一行交它（满载丢弃语义在 sink 侧）。
    /// `None` = 本装配经 router（`SEBAS_AGENT_ROUTER_URL` 已注入，router 已
    /// 记账）→ **不本地记**（双算规避的写入侧分叉，design D1）；装配在
    /// Arc 之后完成，所以走 `RwLock` 注入而非构造参数。
    local_usage: std::sync::RwLock<Option<crate::usage_local::LocalUsageSink>>,
}

impl DispatchNativeBridge {
    pub fn new(manager: Arc<SessionManager>, router: DispatchHandle) -> Arc<Self> {
        Self::with_default(manager, router, false)
    }

    /// 带默认执行体语义的构造（`default_native` = 新 feishu 会话是否走原生）。
    pub fn with_default(
        manager: Arc<SessionManager>,
        router: DispatchHandle,
        default_native: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            manager,
            router,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            default_native,
            local_usage: std::sync::RwLock::new(None),
        })
    }

    /// （add-local-usage-statistics 3.2）装配点注入 native 直连回合的本地
    /// 落账 sink（run.rs 按 `SEBAS_AGENT_ROUTER_URL` 门控后传入；`None` =
    /// 经 router 的装配，不本地记）。桥以 `Arc` 共享、装配晚于构造，所以是
    /// 后注入方法；幂等覆盖（重复 set 以最后一次为准）。sink 是廉价 Clone
    /// （通道发送端 + 查询句柄），按值注入。
    pub fn set_local_usage(&self, sink: Option<crate::usage_local::LocalUsageSink>) {
        *self
            .local_usage
            .write()
            .unwrap_or_else(|e| e.into_inner()) = sink;
    }

    /// 编码 `ChannelKey` 为 router/通道侧形态（复用 router 的共享实现）。
    fn encode(key: &ChannelKey) -> String {
        sebas_dispatch::engine::encode_key(key)
    }

    /// 该 key 是否已是原生会话（在桥的 sessions 表中）。
    fn is_registered(&self, key: &ChannelKey) -> bool {
        let g = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        g.contains_key(&Self::encode(key))
    }

    /// 内核事件 → router 状态。从 pump task 调用。
    /// `key` = 飞书原生会话 key（mapping/事件广播用）；`session_id` = 内核
    /// UUID（`turn_log` 的键，`session_turns` 按此读取）。
    async fn pump(
        bridge: Arc<Self>,
        key: ChannelKey,
        session_id: String,
        mut rx: tokio::sync::broadcast::Receiver<AgentEvent>,
    ) {
        // （add-local-usage-statistics 3.2）回合终态观察：内核的收尾序列是
        // 「终态事件（Finished/Error）先行、SessionSummary 随后」——落账行
        // 的 status/error 从这里取（summary 帧自身不携带终态）。消费即清。
        let mut last_terminal: Option<(u16, Option<String>)> = None;
        while let Ok(ev) = rx.recv().await {
            match ev {
                AgentEvent::TextDelta { delta, .. } => {
                    let entry = TurnEntry::markdown(0, delta.clone());
                    bridge
                        .router
                        .push_transcript_entry(&session_id, entry)
                        .await;
                    // 事件驱动 Updated，让 webui/channel 看到新内容。
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::ToolStart {
                    tool_name,
                    args,
                    tool_use_id,
                    ..
                } => {
                    let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                    let rendered = format!("📖 **{tool_name}**\n```json\n{args_str}\n```");
                    // fold-tool-calls-into-process-tree 3.1：native 载体升为
                    // 一等 tool 条目（`element_type = "tool"` + 结构化标题 +
                    // call id），与 ACP 面同构——不再以 markdown 正文呈现。
                    let mut entry = TurnEntry::tool(0, rendered).with_title(
                        sebas_dispatch::tool_entry_title(false, &tool_name, Some(&args)),
                    );
                    if let Some(id) = &tool_use_id {
                        entry = entry.with_tool_use_id(id.clone());
                    }
                    bridge
                        .router
                        .push_transcript_entry(&session_id, entry)
                        .await;
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::ToolEnd {
                    tool_name,
                    result,
                    tool_use_id,
                    ..
                } => {
                    let rendered = format!("✓ **{tool_name}**\n{result}");
                    // fold-tool-calls-into-process-tree 3.1：结果条目同为一等
                    // tool 条目；ToolEnd wire 无 args → 标题退化 `✓ {tool}`
                    // （与 ACP 面同一口径），call id 与配对调用相等。
                    let mut entry = TurnEntry::tool(0, rendered).with_title(
                        sebas_dispatch::tool_entry_title(true, &tool_name, None),
                    );
                    if let Some(id) = &tool_use_id {
                        entry = entry.with_tool_use_id(id.clone());
                    }
                    bridge
                        .router
                        .push_transcript_entry(&session_id, entry)
                        .await;
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::PermissionRequest {
                    session_id,
                    request_id,
                    tool_name,
                    args,
                    ..
                } => {
                    // 记录 request_id → 内核 session_id（回填用），再把权限请求
                    // 以 AcpEvent 形状送上 router 的权限广播（webui 审查卡中继）。
                    bridge
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(request_id.clone(), session_id);
                    let encoded = Self::encode(&key);
                    bridge
                        .router
                        .publish_native_permission(encoded, request_id, tool_name, args)
                        .await;
                }
                AgentEvent::Finished { .. } => {
                    last_terminal = Some((
                        crate::usage_local::TURN_STATUS_FINISHED,
                        None,
                    ));
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::Error {
                    message, terminal, ..
                } => {
                    // 非 terminal 的「turn cancelled」= 操作者中性取消（499）；
                    // 其余 Error（含 terminal 崩坏）= 失败（500）。
                    let status = if !terminal && message == "turn cancelled" {
                        crate::usage_local::TURN_STATUS_CANCELLED
                    } else {
                        crate::usage_local::TURN_STATUS_FAILED
                    };
                    last_terminal = Some((status, Some(message.clone())));
                    let rendered = format!("⚠ {message}");
                    let entry = TurnEntry::markdown(0, rendered);
                    bridge
                        .router
                        .push_transcript_entry(&session_id, entry)
                        .await;
                    if terminal {
                        bridge.router.fail_native_session(&key).await;
                    }
                }
                AgentEvent::SessionSummary { usage, turn_ms, .. } => {
                    // （add-local-usage-statistics 3.2/D5）native 回合结算一行：
                    // usage 如实透传（None 保 None），终态取上面观察到的收尾；
                    // sink 未装配（经 router 的装配）= 不本地记（双算规避）。
                    if let Some(sink) = bridge
                        .local_usage
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                    {
                        let (status, error) =
                            last_terminal.take().unwrap_or((crate::usage_local::TURN_STATUS_FINISHED, None));
                        sink.record(usage.unwrap_or_default().into_turn_record(
                            crate::usage_local::PROTOCOL_NATIVE,
                            status,
                            turn_ms,
                            error,
                        ));
                    }
                    bridge.router.touch_native_session(&key).await;
                }
                _ => {}
            }
        }
    }
}

impl NativeSessionBridge for DispatchNativeBridge {
    fn is_native(&self, key: &ChannelKey) -> bool {
        // 已登记的原生会话 → 走桥；新会话按默认执行体（default_native）。
        self.is_registered(key) || self.default_native
    }

    fn prompt(self: Arc<Self>, key: ChannelKey, text: String) {
        tokio::spawn(async move {
            let encoded = Self::encode(&key);
            // 已存在 → 续聊。
            let existing = {
                let g = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                g.get(&encoded).cloned()
            };
            if let Some(sess) = existing {
                sess.handle.prompt(text).await;
                return;
            }
            // 新会话：workdir 默认当前目录（飞书原生会话无 project 绑定）。
            let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let handle = self.manager.create_session(workdir);
            let session_id = handle.key.clone();
            {
                let mut g = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                g.insert(
                    encoded.clone(),
                    NativeSession {
                        handle: handle.clone(),
                    },
                );
            }
            // 登记进 router（Active，session_id = 内核 key），广播 Created。
            self.router
                .insert_mapping(key.clone(), session_id.clone())
                .await;
            // 事件泵：内核事件 → router 状态。必须先订阅再首 prompt——
            // 内核在首个 turn 内就可能发 PermissionRequest，晚订阅会丢事件
            // （broadcast 只转发订阅后的事件）。
            let rx = handle.subscribe();
            let pump_key = key.clone();
            tokio::spawn(async move {
                Self::pump(self, pump_key, session_id, rx).await;
            });
            // 首条消息即首 prompt。
            handle.prompt(text).await;
        });
    }

    fn answer_permission(&self, request_id: &str, decision: NativeApprovalDecision) -> bool {
        // 决定词汇已合一（type-session-vocabularies 3.2）：`NativeApprovalDecision` 与
        // `ApprovalAnswer` 是**同一个**共享类型，直投即可，没有手写桥。
        // 未知取值**不得**静默解决泊车审批（spec `agent-driver`）：如实拒绝，
        // 待决表条目保留（审批保持悬空，操作者还能重来）。
        if !decision.is_answerable() {
            eprintln!(
                "native_dispatch_bridge: 审批 {request_id} 收到未知决定 {:?}，已拒绝投递（审批保持悬空）",
                decision.as_str()
            );
            return false;
        }
        // 持有锁的跨度要短：取出内核 session_id 与句柄克隆，锁外异步投递。
        let handle = {
            let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let Some(sid) = pending.get(request_id).cloned() else {
                return false;
            };
            let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            // sessions 按编码 key 索引；内核 session_id 需反查。为免引入
            // 第二张表，这里按 value 扫描（待决请求数量小，可接受）。
            sessions
                .values()
                .find(|s| s.handle.key == sid)
                .map(|s| s.handle.clone())
        };
        let Some(handle) = handle else {
            return false;
        };
        // 投递决定：SessionHandle::answer_permission → ApproverHub::answer。
        // 无匹配请求被内核静默丢弃 → 工具调用 fail-closed 不执行。
        let answer = decision;
        let request_id_owned = request_id.to_string();
        let request_id_spawn = request_id_owned.clone();
        tokio::spawn(async move {
            handle.answer_permission(request_id_spawn, answer).await;
        });
        // 决定已投递：从待决表移除（重复点击不再生效）。
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_id_owned);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_dispatch::state::SessionMap;

    fn manager() -> Arc<SessionManager> {
        let llm = sebas_agent::llm::fake::FakeLlmClient::scripted(vec![
            sebas_agent::llm::fake::FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "ls"}),
            )]),
            sebas_agent::llm::fake::FakeLlmClient::say("done"),
        ]);
        Arc::new(
            SessionManager::new(
                Arc::new(llm),
                sebas_agent::tools::ToolRegistry::with_sandbox(
                    std::time::Duration::from_secs(10),
                    sebas_agent::policy::SandboxMode::Firewall,
                ),
                Default::default(),
            )
            .with_policy(Arc::new(sebas_agent::policy::PolicyEngine::new(
                Default::default(),
            )))
            .with_approver(sebas_agent::policy::ApproverHub::new()),
        )
    }

    #[tokio::test]
    async fn bridge_prompts_and_registers_mapping() {
        let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let bridge = DispatchNativeBridge::new(manager(), router.clone());

        let key = ChannelKey::new("feishu", "agent-f-1");
        bridge.prompt(key.clone(), "go".into());

        // router 应出现该原生会话的 mapping（Active + 内核 session_id）。
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if router.session_exists(&key).await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("native session should be registered");
        let info = router.session_info_for(&key).await.expect("mapping exists");
        assert!(info.session_id.is_some(), "native session_id should be set");
    }

    /// fold-tool-calls-into-process-tree 3.1：native 载体的工具痕迹升为
    /// 一等 tool 条目——`element_type == tool`、结构化标题（`Read · <path>`
    /// 形态，复用 sebas-dispatch 的键序规则）、调用与结果携带相等 call id。
    #[tokio::test]
    async fn tool_traces_land_as_first_class_tool_entries_with_titles_and_ids() {
        let llm = sebas_agent::llm::fake::FakeLlmClient::scripted(vec![
            sebas_agent::llm::fake::FakeLlmClient::call_tools(vec![(
                "tc-read-1",
                "read",
                serde_json::json!({"path": "src/main.rs"}),
            )]),
            sebas_agent::llm::fake::FakeLlmClient::say("done"),
        ]);
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(llm),
                sebas_agent::tools::ToolRegistry::with_sandbox(
                    std::time::Duration::from_secs(10),
                    sebas_agent::policy::SandboxMode::Firewall,
                ),
                Default::default(),
            )
            .with_policy(Arc::new(sebas_agent::policy::PolicyEngine::new(
                Default::default(),
            ))),
        );
        let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let bridge = DispatchNativeBridge::new(manager, router.clone());

        let key = ChannelKey::new("feishu", "agent-f-tool");
        bridge.prompt(key.clone(), "go".into());

        // 等回合收尾（转录里出现正文 "done" 即内核两个 model call 都已落账）。
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(turns) = router.session_turns(&key, 0).await
                    && turns.iter().any(|t| t.content.contains("done"))
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("turn should finish");

        let turns = router.session_turns(&key, 0).await.expect("transcript");
        let tools: Vec<_> = turns
            .iter()
            .filter(|t| {
                t.element_type == sebas_domain::session::TurnElementType::Tool
            })
            .collect();
        assert_eq!(tools.len(), 2, "call + result both land as tool entries");
        // 调用条目：一等 tool + `Read · src/main.rs` 形态标题 + call id。
        assert_eq!(
            tools[0].tool_use_id.as_deref(),
            Some("tc-read-1"),
            "invocation carries the upstream call id"
        );
        assert_eq!(
            tools[0].title.as_deref(),
            Some("read · src/main.rs"),
            "structured title reuses the dispatch key-order rule"
        );
        // 结果条目：同一 id；ToolEnd 无 args → 标题退化 `✓ read`。
        assert_eq!(tools[1].tool_use_id.as_deref(), Some("tc-read-1"));
        assert_eq!(tools[1].title.as_deref(), Some("✓ read"));
    }

    // ---- add-local-usage-statistics 2.4：双算规避的两种装配分叉 ----
    //
    // run.rs 的装配门控（`native_records_locally_from_env`）只有两个出口：
    // 直连装配注入 sink（Some）→ pump 落行；经 router 的装配注入 None →
    // pump 零动作（router 已记）。ACP 会话恒落的半边由
    // sebas-dispatch/tests/local_usage_capture_test.rs 钉住（钩子无条件装配）。

    /// 桥 + 真实本地账本的一轮回合 harness：fake 内核一段带 usage 的文本
    /// 回合，等收尾后从 `db` 路径读回落好的行。
    async fn bridge_turn_rows(
        sink: Option<crate::usage_local::LocalUsageSink>,
        db: &std::path::Path,
    ) -> Vec<crate::usage_local::LocalUsageRow> {
        let llm = sebas_agent::llm::fake::FakeLlmClient::scripted(vec![
            sebas_agent::llm::fake::FakeLlmClient::say_with_usage("done", 3, 9),
        ]);
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(llm),
                sebas_agent::tools::ToolRegistry::with_sandbox(
                    std::time::Duration::from_secs(10),
                    sebas_agent::policy::SandboxMode::Firewall,
                ),
                Default::default(),
            )
            .with_policy(Arc::new(sebas_agent::policy::PolicyEngine::new(
                Default::default(),
            ))),
        );
        let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let bridge = DispatchNativeBridge::new(manager, router.clone());
        bridge.set_local_usage(sink);

        let key = ChannelKey::new("feishu", "agent-f-usage");
        bridge.prompt(key.clone(), "go".into());

        // 等回合收尾（转录里出现正文 "done" 即 summary 已发射——落账在
        // 同一 pump 循环内先行完成）。
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(turns) = router.session_turns(&key, 0).await
                    && turns.iter().any(|t| t.content.contains("done"))
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("turn should finish");
        // writer 是异步的：再给一点提交时间。
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        use sebas_db::record::Record;
        let conn = sebas_db::conn::open(db).expect("open local usage db");
        let mut stmt = conn
            .prepare("SELECT id, key, protocol, model, provider, upstream_model, status,
                             latency_ms, ttft_ms, input_tokens, output_tokens,
                             cache_read_tokens, cache_creation_tokens, error, ts
                      FROM local_usage_records ORDER BY id")
            .expect("prepare");
        stmt.query_map([], crate::usage_local::LocalUsageRow::from_row)
            .expect("query")
            .collect::<sebas_db::rusqlite::Result<Vec<_>>>()
            .expect("collect")
    }

    #[tokio::test]
    async fn direct_native_turn_lands_one_local_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("usage_local.db");
        let sink = crate::usage_local::LocalUsageSink::spawn(
            &db,
            sebas_router::usage::RetentionPolicy {
                prune_interval_secs: 0,
                ..Default::default()
            },
        )
        .expect("spawn sink");
        let rows = bridge_turn_rows(Some(sink), &db).await;
        assert_eq!(rows.len(), 1, "直连回合恰好一行, got {}", rows.len());
        assert_eq!(rows[0].protocol, "native");
        assert_eq!(rows[0].input_tokens, Some(3));
        assert_eq!(rows[0].output_tokens, Some(9));
        assert_eq!(rows[0].status, 200, "Finished = 完成");
    }

    /// 无 usage 的直连回合（task 3.2）：仍落一行（只计请求数），token 逐
    /// 字段 NULL（不以全零冒充「已上报 0」）。
    #[tokio::test]
    async fn native_turn_without_usage_lands_a_request_only_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("usage_local.db");
        let sink = crate::usage_local::LocalUsageSink::spawn(
            &db,
            sebas_router::usage::RetentionPolicy {
                prune_interval_secs: 0,
                ..Default::default()
            },
        )
        .expect("spawn sink");
        // harness 固定用 say_with_usage；这里手工装配一轮回合（纯文本、零
        // usage）以钉「无 usage 也有一行」的另一半。
        let llm = sebas_agent::llm::fake::FakeLlmClient::scripted(vec![
            sebas_agent::llm::fake::FakeLlmClient::say("plain done"),
        ]);
        let manager = Arc::new(
            SessionManager::new(
                Arc::new(llm),
                sebas_agent::tools::ToolRegistry::with_sandbox(
                    std::time::Duration::from_secs(10),
                    sebas_agent::policy::SandboxMode::Firewall,
                ),
                Default::default(),
            )
            .with_policy(Arc::new(sebas_agent::policy::PolicyEngine::new(
                Default::default(),
            ))),
        );
        let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let bridge = DispatchNativeBridge::new(manager, router.clone());
        bridge.set_local_usage(Some(sink));

        let key = ChannelKey::new("feishu", "agent-f-nousage");
        bridge.prompt(key.clone(), "go".into());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(turns) = router.session_turns(&key, 0).await
                    && turns.iter().any(|t| t.content.contains("plain done"))
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("turn should finish");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        use sebas_db::record::Record;
        let conn = sebas_db::conn::open(&db).expect("open local usage db");
        let mut stmt = conn
            .prepare("SELECT id, key, protocol, model, provider, upstream_model, status,
                             latency_ms, ttft_ms, input_tokens, output_tokens,
                             cache_read_tokens, cache_creation_tokens, error, ts
                      FROM local_usage_records ORDER BY id")
            .expect("prepare");
        let rows = stmt
            .query_map([], crate::usage_local::LocalUsageRow::from_row)
            .expect("query")
            .collect::<sebas_db::rusqlite::Result<Vec<_>>>()
            .expect("collect");
        assert_eq!(rows.len(), 1, "无 usage 回合也有一行（请求数），got {rows:?}");
        assert_eq!(rows[0].input_tokens, None, "未上报保 NULL，不冒充零");
        assert_eq!(rows[0].output_tokens, None);
        assert_eq!(rows[0].status, 200);
    }

    #[tokio::test]
    async fn router_routed_native_assembly_lands_no_local_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("usage_local.db");
        // 账本本身在库里（不是「库不存在」）：装配为 None 只是 pump 不落行。
        // sink 全程保活（库与表在场），只是不交给桥。
        let sink = crate::usage_local::LocalUsageSink::spawn(
            &db,
            sebas_router::usage::RetentionPolicy {
                prune_interval_secs: 0,
                ..Default::default()
            },
        )
        .expect("spawn sink");
        let rows = bridge_turn_rows(None, &db).await;
        assert!(
            rows.is_empty(),
            "经 router 的装配不本地记（router 已记），got {rows:?}"
        );
        drop(sink);
    }
}
