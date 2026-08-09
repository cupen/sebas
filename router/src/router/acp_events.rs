//! ACP 事件 → `Out` 指令的翻译（权限请求、terminal 错误、流式事件落地）。
//!
//! `RouterHandle` 的 impl 延续块（子模块可访问私有字段），从 router.rs 拆出。

use super::{Out, RouterHandle, extract_session_id};
use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::cards::render_permission_card;

impl RouterHandle {
    /// Dispatch an inbound `AcpEvent`, extracting the session_id from the
    /// event payload and forwarding to `apply_event_to_out`.
    pub async fn dispatch_acp_event(&self, event: AcpEvent) {
        let session_id = extract_session_id(&event).to_owned();
        self.apply_event_to_out(session_id, &event).await;
    }

    /// apply_event_to_out：同步薄封装（apply_event + flush_card 即时出卡）。
    ///
    /// **Spec §6 偏差**：spec §6 说「dispatch_acp_event 改为调 apply_event 不发
    /// Out」，但 spec §9 要求 router_test/e2e_test/terminal_error_test 零改动通过
    /// —— 这些测试调 dispatch_acp_event 并断言立即收到 UpdateCard。故本计划保留
    /// dispatch_acp_event → apply_event_to_out（同步 flush），仅把 **pump** 从
    /// dispatch_acp_event 改为 apply_event + debounce + flush_card（Task 6）。
    pub async fn apply_event_to_out(&self, session_id: String, event: &AcpEvent) {
        match event {
            AcpEvent::PermissionRequest {
                session_id,
                request_id,
                tool_name,
                args,
            } => {
                // Resolve the SessionKey that owns this session so Feishu has a
                // real `receive_id`. Without this the card would carry an empty
                // chat_id and Feishu rejects it.
                let Some(key) = self.map.lookup_key_by_session(session_id).await else {
                    tracing::warn!(%session_id, "no SessionKey for permission request; dropping card");
                    return;
                };
                // Auto-approve if the user previously clicked "本会话不再
                // 询问" in this chat. No card, no user click — the bridge
                // gets the same AllowSession reply as a manual click
                // would have produced.
                if self.allowlist.is_allowed(&key, tool_name, args).await {
                    tracing::info!(
                        %session_id, %tool_name, %request_id,
                        "permission auto-approved by session allowlist"
                    );
                    self.emit(Out::SendAcp {
                        session_id: session_id.clone(),
                        cmd: AcpCommand::PermissionReply {
                            session_id: session_id.clone(),
                            request_id: request_id.clone(),
                            decision: Decision::AllowSession,
                        },
                    })
                    .await;
                    return;
                }
                let card = render_permission_card(session_id, request_id, tool_name, args);
                // 话题会话：权限卡回复到话题根消息，保证聚合在原话题（Q5）。
                // 主线保持现状（root_id=None，Q7）。
                let root_id = if key.thread_id.is_some() {
                    self.reply_target(&key).await
                } else {
                    None
                };
                self.emit(Out::SendCard {
                    key,
                    card: serde_json::to_value(&card).expect("permission card serializes"),
                    msg_id: None,
                    // Mark this card for in-place update on click. The dispatcher
                    // records the Feishu message_id keyed by request_id so a
                    // later button click can flip the card to "已允许/已拒绝"
                    // or "请求已过期" instead of leaving the user staring at
                    // a stale prompt they keep re-clicking.
                    perm_request_id: Some(request_id.clone()),
                    // Stash the call metadata for the click handler.
                    perm_meta: Some((tool_name.clone(), args.clone())),
                    // Permission cards are fire-and-forget (no threading).
                    root_id,
                })
                .await;
            }
            AcpEvent::Error { terminal: true, .. } => {
                // terminal Error 并入累积模型（spec §8）：apply_event（置 ❌ + append
                // 错误正文，保留死前 transcript）→ flush_card → 换 reaction →
                // remove_by_session → drop_card。drop_card 无条件执行（无论
                // SessionKey 是否还在都该清 CardState 防无界增长）。
                let react = self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
                if let Some(emoji) = react {
                    self.emit_reaction(session_id.as_str(), emoji).await;
                }
                let sid = session_id.as_str();
                if let Some(key) = self.map.lookup_key_by_session(sid).await {
                    self.allowlist.clear(&key).await;
                    self.reply_targets.clear(&key).await;
                    self.map.remove_by_session(sid).await;
                }
                self.drop_card(sid).await;
            }
            _ => {
                // 流式事件 + Finished + 非 terminal Error：apply_event（状态）+ flush_card（同步出卡）。
                // FSM emoji 转移时紧跟一个 React（先出卡，后换 reaction）。
                let react = self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
                // Emit the terminal reaction BEFORE draining: once the drained
                // turn's SendCard is dispatched, MsgIdMap points at the NEW
                // card, so a later React would land the DONE/FAILED emoji on
                // the wrong card.
                if let Some(emoji) = react {
                    self.emit_reaction(session_id.as_str(), emoji).await;
                }
                if let Some(key) = self.map.lookup_key_by_session(session_id.as_str()).await {
                    self.drain_queue_if_terminal(&key, session_id.as_str())
                        .await;
                }
            }
        }
    }
}
