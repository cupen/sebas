//! 出站分发：把 router 产出的 `Out` 指令落实为飞书/ACP 副作用。
//!
//! 从 `run.rs` 拆出；只被出站泵（`crate::run`）调用。

use crate::config::Config;
use crate::reactions::{ReactPlan, ReactionTracker};
use crate::session_boot::{
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts, spawn_acp_pump,
    wire_session_card_and_pump,
};
use acp_claude::manager::SessionManager;
use feishu::client::FeishuClient;
use router::router::{Out, RouterHandle};
use std::sync::Arc;
use tracing::{debug, info, warn};

// 参数即 outbound 共享上下文（client/http/tokens/cfg/router/mgr/reactions），
// 打包 struct 只会给每个 match arm 增加 `ctx.` 噪音。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_out(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    tokens: &feishu::client::TokenManager,
    cfg: &Config,
    router: &RouterHandle,
    mgr: &Arc<SessionManager>,
    reactions: &ReactionTracker,
    out: Out,
) -> anyhow::Result<()> {
    match out {
        Out::SendCard {
            key,
            card,
            msg_id,
            perm_request_id,
            perm_meta,
            root_id,
        } => {
            // The MsgIdMap is keyed by session_id (never chat_id) so that
            // `UpdateCard`/`React`, which only know the session_id, can resolve
            // the message_id. Only record when a session_id is supplied; plain
            // cards (permission prompts, help) don't need to be updated later.
            let new_id = feishu
                .send_card(http, tokens, &key, card, root_id.as_deref())
                .await?;
            if let (false, Some(session_id)) = (new_id.is_empty(), msg_id) {
                router.record_root_msg_id(session_id, new_id.clone()).await;
                debug!(message_id = %new_id, "recorded card msg_id");
            }
            // Permission cards are tracked by request_id so a later button
            // click can flip them in place (resolved/expired). We also stash
            // (tool_name, args) so "Allow session" can register an entry in
            // the session allowlist for auto-approving future calls.
            if let (false, Some(req_id)) = (new_id.is_empty(), perm_request_id) {
                let (tool_name, args) = perm_meta.unwrap_or_default();
                router
                    .record_perm_card_msg_id(
                        req_id.clone(),
                        key.clone(),
                        new_id.clone(),
                        tool_name,
                        args,
                    )
                    .await;
                debug!(%req_id, message_id = %new_id, "recorded perm card msg_id");
            }
        }
        Out::UpdateCard { session_id, card } => {
            if let Some(message_id) = router.root_msg_id(&session_id).await {
                feishu.update_card(http, tokens, &message_id, card).await?;
            } else {
                debug!(?session_id, "no root msg_id recorded; skipping update");
            }
        }
        Out::UpdateCardByMsgId { key, msg_id, card } => {
            // PATCH the card by its Feishu message_id (no session lookup
            // needed). Used for permission-card click feedback where the
            // session is still alive but we just want to flip the prompt in
            // place. Failure is non-fatal: a stale msg_id is a no-op on
            // Feishu's side.
            if let Err(e) = feishu.update_card(http, tokens, &msg_id, card).await {
                warn!(%msg_id, error=%e, "perm card update failed");
            }
            let _ = key; // chat context — currently unused; the API only needs msg_id
        }
        Out::AckMsg { message_id, emoji } => {
            // Fire-and-forget acknowledgment reaction on the user's message.
            // No tracking needed — unlike Out::React, this is a one-shot
            // notification that does not need to be swapped later.
            if let Err(e) = feishu.react(http, tokens, &message_id, &emoji).await {
                warn!(%message_id, error=%e, "ack reaction failed");
            }
        }
        Out::React { session_id, emoji } => {
            // 状态 reaction 优先落在用户输入消息上（本次功能）；无输入消息
            // 时（WebUI /new/replay）回退到会话卡片，保持旧行为。
            let input_id = router.input_msg_id(&session_id).await;
            let target = match input_id {
                Some(id) => Some(id),
                None => router.root_msg_id(&session_id).await,
            };
            if let Some(message_id) = target {
                match reactions.plan(&session_id, &emoji).await {
                    ReactPlan::Skip => {}
                    ReactPlan::ReactOnly => {
                        let rid = feishu.react(http, tokens, &message_id, &emoji).await?;
                        reactions.record(&session_id, emoji, rid).await;
                    }
                    ReactPlan::Swap { unreact_id } => {
                        // Best-effort: a stale reaction already gone is not fatal,
                        // but failing to add the new one would strand the state.
                        if let Err(e) = feishu.unreact(http, tokens, &message_id, &unreact_id).await
                        {
                            warn!(%session_id, "unreact before swap failed (continuing): {e}");
                        }
                        let rid = feishu.react(http, tokens, &message_id, &emoji).await?;
                        reactions.record(&session_id, emoji, rid).await;
                    }
                }
            } else {
                debug!(?session_id, "no input/card msg_id; skipping react");
            }
        }
        Out::SpawnAcp {
            key,
            prompt,
            input_msg_id,
        } => {
            let claude = &cfg.acp.claude;
            // 1) Spawn the claude subprocess, mint a session_id, send the
            //    initial prompt, and flip the router's Spawning placeholder
            //    to Active (draining queued prompts). On failure (missing
            //    binary, handshake timeout, or prompt send failure): drop the
            //    placeholder and show an ❌ card instead of a silent log line
            //    (spec §4.1 "ACP spawn failure"). create_session and
            //    CreateSession-prompt failures share the same Err branch —
            //    both mean the session is unusable.
            let (session_id, pending, rx) = match acp_spawn_and_activate(
                mgr,
                router,
                &key,
                &prompt,
                &claude.path,
                claude.args.clone(),
                claude.work_dir.clone(),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 启动失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    if let Err(e2) = feishu
                        .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                        .await
                    {
                        warn!(?e2, "failed to send spawn-failure card");
                    }
                    warn!(?e, "create_session failed");
                    return Ok(());
                }
            };
            // 把触发本次 spawn 的输入消息 id 记录到 session，供状态 reaction
            // 落在用户消息上（替换卡片 reaction；见 `emit_reaction` 落点）。
            if let Some(input_id) = &input_msg_id {
                router
                    .record_input_msg_id(session_id.clone(), input_id.clone())
                    .await;
            }
            wire_session_card_and_pump(
                feishu, http, tokens, cfg, router, mgr, reactions, key, session_id, prompt,
                pending, rx, input_msg_id,
            )
            .await?;
        }
        Out::WebSpawn {
            key,
            prompt,
            project_dir,
        } => {
            let claude = &cfg.acp.claude;
            // Web-originated spawn: create the ACP session and wire the pump,
            // but skip the Feishu send_card / react operations. Card content
            // is still accumulated in CardStateMap and readable via the WebUI.
            // `project_dir` takes precedence over the config default.
            let (session_id, pending, rx) = match acp_spawn_and_activate(
                mgr,
                router,
                &key,
                &prompt,
                &claude.path,
                claude.args.clone(),
                project_dir.or_else(|| claude.work_dir.clone()),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    warn!(?e, "web_spawn: acp_spawn_and_activate failed");
                    return Ok(());
                }
            };
            // Seed card state and wire the pump (no Feishu card operations).
            router.seed_card(session_id.clone(), prompt.clone()).await;
            spawn_acp_pump(rx, router.clone(), session_id.clone());
            // Flush prompts queued during spawn.
            if let Err(e) = flush_pending_prompts(mgr, &session_id, pending).await {
                warn!(?e, "web_spawn: flush_pending_prompts failed");
            }
        }
        Out::SpawnResume {
            key,
            session_id: old_sid,
            prompt,
            input_msg_id,
        } => {
            let claude = &cfg.acp.claude;
            // Lazy respawn of a restored mapping (spec §3.3e): claude-native
            // `resume` of the persisted id; the manager transparently falls
            // back to a fresh session when the conversation is gone
            // (sebas-dk8.4). `resumed` says which happened — on fallback the
            // old context did NOT carry over, so tell the user instead of
            // silently continuing fresh.
            let (session_id, pending, rx, resumed) = match acp_resume_and_activate(
                mgr,
                router,
                &key,
                &old_sid,
                &prompt,
                &claude.path,
                claude.args.clone(),
                claude.work_dir.clone(),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 恢复失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    if let Err(e2) = feishu
                        .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                        .await
                    {
                        warn!(?e2, "failed to send resume-failure card");
                    }
                    warn!(?e, %old_sid, "resume_session failed");
                    return Ok(());
                }
            };
            if !resumed {
                info!(%old_sid, %session_id, "old session could not be loaded; continued as fresh session");
                let card = feishu::cards::render_session_lost_card();
                if let Err(e2) = feishu
                    .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                    .await
                {
                    warn!(?e2, "failed to send session-lost notice");
                }
            }
            // 与 SpawnAcp 一致：记录输入消息 id，供状态 reaction 落在用户消息上。
            if let Some(input_id) = &input_msg_id {
                router
                    .record_input_msg_id(session_id.clone(), input_id.clone())
                    .await;
            }
            wire_session_card_and_pump(
                feishu, http, tokens, cfg, router, mgr, reactions, key, session_id, prompt,
                pending, rx, input_msg_id,
            )
            .await?;
        }
        Out::SendAcp { session_id, cmd } => {
            mgr.send(&session_id, cmd).await?;
        }
        Out::HelpText { key } => {
            info!(?key, "send help (no-op: help text not implemented)");
        }
        Out::PlainText { key, content } => {
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogUpgrade { key, dev, dry_run } => {
            let request = crate::watchdog::control_rpc::RpcControlRequest::Update { dev, dry_run };
            let content = submit_watchdog_control(request, "升级").await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogRollback { key } => {
            let request = crate::watchdog::control_rpc::RpcControlRequest::Rollback { dry_run: false };
            let content = submit_watchdog_control(request, "回滚").await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogRestart { key } => {
            let content = submit_watchdog_control(
                crate::watchdog::control_rpc::RpcControlRequest::RestartCore,
                "重启 core",
            )
            .await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogServices { key } => {
            let content = submit_watchdog_control(
                crate::watchdog::control_rpc::RpcControlRequest::ServiceStatus,
                "服务状态查询",
            )
            .await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
    }
    Ok(())
}

async fn submit_watchdog_control(
    request: crate::watchdog::control_rpc::RpcControlRequest,
    label: &str,
) -> String {
    let secret = match std::env::var("SEBAS_CONTROL_SECRET") {
        Ok(secret) if !secret.is_empty() => secret,
        _ => return format!("{label}请求失败: 当前进程未获得 watchdog control secret"),
    };

    let envelope = crate::watchdog::control_rpc::ControlEnvelope {
        version: 1,
        request_id: format!("feishu_{label}"),
        secret,
        actor: crate::watchdog::control_rpc::RpcActor::Cli { uid: current_uid() },
        request,
    };

    match crate::watchdog::control_rpc::request(
        &crate::watchdog::control_rpc::default_socket_path(),
        &envelope,
    )
    .await
    {
        Ok(crate::watchdog::control_rpc::RpcControlResponse::Accepted {
            operation_id,
            status,
        }) => format!("已提交{label}请求给 watchdog: {operation_id} ({status})"),
        Ok(crate::watchdog::control_rpc::RpcControlResponse::Rejected { code, message }) => {
            format!("{label}请求被拒绝: {code}: {message}")
        }
        Ok(crate::watchdog::control_rpc::RpcControlResponse::Events { events }) => {
            if events.is_empty() {
                "watchdog 暂无服务事件".to_string()
            } else {
                let mut lines = vec!["watchdog 最近服务事件:".to_string()];
                for event in events.iter().rev().take(5).rev() {
                    lines.push(format!(
                        "#{} [{}] {} {}",
                        event.seq, event.kind, event.operation_id, event.public_message
                    ));
                }
                lines.join("\n")
            }
        }
        Ok(crate::watchdog::control_rpc::RpcControlResponse::Services { services }) => {
            if services.is_empty() {
                "watchdog 暂无服务状态信息".to_string()
            } else {
                let mut lines = vec!["watchdog 服务状态:".to_string()];
                for svc in &services {
                    lines.push(format!(
                        "- {}: {} (期望: {})",
                        svc.name, svc.status, svc.desired
                    ));
                }
                lines.join("\n")
            }
        }
        Err(e) => format!("{label}请求失败: {e}"),
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}
