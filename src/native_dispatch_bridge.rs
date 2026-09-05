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

use sebas_agent::policy::ApprovalAnswer;
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
        })
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
                AgentEvent::ToolStart { tool_name, args, .. } => {
                    let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                    let rendered = format!("📖 **{tool_name}**\n```json\n{args_str}\n```");
                    let entry = TurnEntry::markdown(0, rendered);
                    bridge
                        .router
                        .push_transcript_entry(&session_id, entry)
                        .await;
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::ToolEnd { tool_name, result, .. } => {
                    let rendered = format!("✓ **{tool_name}**\n{result}");
                    let entry = TurnEntry::markdown(0, rendered);
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
                        .publish_native_permission(encoded, request_id, tool_name, args);
                }
                AgentEvent::Finished { .. } => {
                    bridge.router.touch_native_session(&key).await;
                }
                AgentEvent::Error { message, terminal, .. } => {
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
            self.router.insert_mapping(key.clone(), session_id.clone()).await;
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
        let answer = match decision {
            NativeApprovalDecision::AllowOnce => ApprovalAnswer::AllowOnce,
            NativeApprovalDecision::AllowSession => ApprovalAnswer::AllowSession,
            NativeApprovalDecision::Deny => ApprovalAnswer::Deny,
            NativeApprovalDecision::Escalate { reason } => ApprovalAnswer::Escalate { reason },
        };
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
}