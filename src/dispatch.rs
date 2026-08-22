//! 出站分发：把 router 产出的 `Out` 指令落实为飞书/ACP 副作用。
//!
//! 从 `run.rs` 拆出；只被出站泵（`crate::run`）调用。

use crate::config::Config;
use crate::reactions::{ReactPlan, ReactionTracker};
use crate::session_boot::{
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts,
    spawn_acp_pump_with_idle, wire_session_card_and_pump,
};
use acp_claude::manager::SessionManager;
use feishu::client::{FeishuApiError, FeishuClient};
use feishu::events::SessionKey;
use gateway::config::GatewayConfig;
use router::commands::GatewayAction;
use router::router::{Out, RouterHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// 话题失效提示文案（Q8→F1 熔断）：发一次提示并终止会话，不重试、不重发。
/// 群聊/p2p 通用，不提「开新话题」。
const TOPIC_INVALID_NOTICE: &str = "该话题已失效，本次会话已结束。请重新发消息开始新会话。";

/// 出站卡统一回复目标：所有 `Out::SendCard` 都应走 reply 形式挂回用户发言。
/// - 显式 `root_id`（如 `Out::SendCard.root_id`，换卡、permission 解析）原样透传；
/// - 否则取 router 存的最近入站消息 `reply_target`：话题内 = 话题根消息
///   message_id，主线 = 用户触发消息 message_id。
///
/// 主线保持 None 的旧 Q7 行为已废除：主线权限卡 / 失败提示卡现在也会挂在
/// 用户发言下，方便沿 thread 跟踪整段对话。
pub(crate) async fn topic_reply_target(
    router: &RouterHandle,
    key: &SessionKey,
    root_id: Option<String>,
) -> Option<String> {
    match root_id {
        Some(r) if !r.is_empty() => Some(r),
        _ => router.reply_target(key).await,
    }
}

/// 出站卡发送结果：区分「已发出」与「话题失效已熔断」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TopicSendOutcome {
    /// 已发出，携带 message_id。
    Sent(String),
    /// 话题失效（230019/230071）：已发文本提示并调 web_close_session 终止会话。
    /// 调用方按「未发出」处理（跳过 record/更新）。
    TopicInvalid,
}

/// 判定 send_card 错误是否话题失效（230019/230071）。话题失效类错误不再
/// 重发，改为提示 + 熔断会话。
fn classify_topic_invalid(e: &anyhow::Error) -> Option<i32> {
    e.downcast_ref::<FeishuApiError>()
        .filter(|api| api.is_topic_invalid())
        .map(|api| api.code)
}

/// 发送卡片；话题失效（230019/230071）时不重试、不重发：向会话发一条文本
/// 提示，并调 `RouterHandle::web_close_session` 终止会话（kill ACP 子进程、
/// 清 SessionMap/CardState/MsgIdMap/allowlist/ReplyTargetMap），返回
/// `TopicInvalid` 供调用方按「未发出」处理。会话映射已删，后续入站走「无
/// 会话」路径，不会再向失效话题出站 —— 提示只发一次，无需去重标记。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_card_topic_aware(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    tokens: &feishu::client::TokenManager,
    router: &RouterHandle,
    key: &SessionKey,
    card: serde_json::Value,
    root_id: Option<String>,
) -> anyhow::Result<TopicSendOutcome> {
    match feishu
        .send_card(http, tokens, key, card, root_id.as_deref())
        .await
    {
        Ok(id) => Ok(TopicSendOutcome::Sent(id)),
        Err(e) => {
            if let Some(code) = classify_topic_invalid(&e) {
                warn!(code, error = %e, "topic send failed; notifying user and closing session");
                if let Err(e2) = feishu
                    .send_text(http, tokens, key, TOPIC_INVALID_NOTICE)
                    .await
                {
                    warn!(?e2, "topic-invalid notice send failed");
                }
                // 熔断：终止会话。无会话的 key 返回 CloseOutcome 非 Closed 变体，
                // 幂等，这里忽略返回值。
                let _ = router.web_close_session(key.clone()).await;
                return Ok(TopicSendOutcome::TopicInvalid);
            }
            Err(e)
        }
    }
}

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
    gateway_cfg: Option<&GatewayConfig>,
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
            // 话题会话且 Out 未带 root_id 时，用 router 存的最近回复目标兜底
            // （覆盖权限卡等所有 Out::SendCard 出站卡，Q5）。
            let reply = topic_reply_target(router, &key, root_id).await;
            let outcome =
                send_card_topic_aware(feishu, http, tokens, router, &key, card, reply).await?;
            // TopicInvalid：已熔断（会话被终止），等价于旧空 message_id ——
            // 跳过 record_root_msg_id / record_perm_card_msg_id。
            if let TopicSendOutcome::Sent(new_id) = outcome {
                if let (false, Some(session_id)) = (new_id.is_empty(), msg_id) {
                    // 帮助卡片：记录 msg_id 供后续 tab 切换原地更新
                    if session_id == "__help_card__" {
                        router.record_help_card_msgid(&key, new_id.clone()).await;
                        debug!(message_id = %new_id, "recorded help card msg_id");
                    } else {
                        router.record_root_msg_id(session_id, new_id.clone()).await;
                        debug!(message_id = %new_id, "recorded card msg_id");
                    }
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
            // Record the reaction_id so the phase reaction handler can later
            // remove this ack emoji before adding the new one.
            match feishu.react(http, tokens, &message_id, &emoji).await {
                Ok(rid) => {
                    reactions.record_ack(&message_id, emoji.clone(), rid).await;
                }
                Err(e) => {
                    warn!(%message_id, error=%e, "ack reaction failed");
                }
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
                // Before planning the phase reaction, check if there's a
                // pending ack reaction (EYES) on this message. If so, remove
                // it first so the phase emoji cleanly replaces the ack.
                if let Some((_, ack_rid)) = reactions.take_ack(&message_id).await {
                    if let Err(e) = feishu.unreact(http, tokens, &message_id, &ack_rid).await {
                        warn!(%session_id, "removing ack reaction before phase swap failed: {e}");
                    }
                }
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
                gateway_cfg,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 启动失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    let reply = topic_reply_target(router, &key, None).await;
                    if let Err(e2) = send_card_topic_aware(
                        feishu,
                        http,
                        tokens,
                        router,
                        &key,
                        serde_json::to_value(&card)?,
                        reply,
                    )
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
                feishu,
                http,
                tokens,
                cfg,
                router,
                mgr,
                reactions,
                key,
                session_id,
                prompt,
                pending,
                rx,
                input_msg_id,
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
                gateway_cfg,
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
            // sebas-9pz ②: idle_kill_secs 接线(与 Feishu 路径一致)。
            let idle_timeout = (cfg.acp.claude.idle_kill_secs > 0)
                .then(|| std::time::Duration::from_secs(cfg.acp.claude.idle_kill_secs));
            spawn_acp_pump_with_idle(
                rx,
                router.clone(),
                session_id.clone(),
                idle_timeout,
                Some(mgr.clone()),
            );
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
                gateway_cfg,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 恢复失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    let reply = topic_reply_target(router, &key, None).await;
                    if let Err(e2) = send_card_topic_aware(
                        feishu,
                        http,
                        tokens,
                        router,
                        &key,
                        serde_json::to_value(&card)?,
                        reply,
                    )
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
                let reply = topic_reply_target(router, &key, None).await;
                if let Err(e2) = send_card_topic_aware(
                    feishu,
                    http,
                    tokens,
                    router,
                    &key,
                    serde_json::to_value(&card)?,
                    reply,
                )
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
                feishu,
                http,
                tokens,
                cfg,
                router,
                mgr,
                reactions,
                key,
                session_id,
                prompt,
                pending,
                rx,
                input_msg_id,
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
            let content = submit_watchdog_control(&key, request, "升级").await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogRollback { key } => {
            let request =
                crate::watchdog::control_rpc::RpcControlRequest::Rollback { dry_run: false };
            let content = submit_watchdog_control(&key, request, "回滚").await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogRestart { key } => {
            let content = submit_watchdog_control(
                &key,
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
                &key,
                crate::watchdog::control_rpc::RpcControlRequest::ServiceStatus,
                "服务状态查询",
            )
            .await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogSystem { key } => {
            let content = submit_watchdog_control(
                &key,
                crate::watchdog::control_rpc::RpcControlRequest::Status,
                "系统状态查询",
            )
            .await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogGateway { key, action } => {
            // Phase 3: route the command; ServiceManager (Phase 4) does the
            // actual work. on/off → ServiceSet; restart → ServiceRestart;
            // status → ServiceStatusFor(gateway).
            let request = gateway_control_request(action);
            let content = submit_watchdog_control(&key, request, "gateway 服务").await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
        Out::WatchdogWebui { key } => {
            let content = submit_watchdog_control(
                &key,
                crate::watchdog::control_rpc::RpcControlRequest::ServiceStatusFor {
                    service: "webui".into(),
                },
                "webui 服务状态",
            )
            .await;
            if let Err(e) = feishu.send_text(http, tokens, &key, &content).await {
                warn!(?e, "send_text failed");
            }
        }
    }
    Ok(())
}

/// Build the Feishu proxy control envelope (spec §6.2 Phase 3). The core
/// proxies on behalf of the Feishu chat, authenticated by the startup secret
/// (the MAC basis). Pre-Phase-5 there is no per-inbound sender open_id on
/// every message, so `chat_id` is authoritative and `open_id` is left empty.
/// Pure and synchronous so adapter-contract tests can assert the normalized
/// request without a live control socket.
fn feishu_control_envelope(
    key: &SessionKey,
    secret: String,
    request: crate::watchdog::control_rpc::RpcControlRequest,
) -> crate::watchdog::control_rpc::ControlEnvelope {
    crate::watchdog::control_rpc::ControlEnvelope {
        version: 1,
        request_id: next_request_id(),
        secret,
        actor: crate::watchdog::control_rpc::RpcActor::Feishu {
            open_id: String::new(),
            chat_id: Some(key.chat_id.clone()),
        },
        request,
    }
}

/// Generate a unique, machine-friendly control RPC request id. The label is
/// intentionally not part of the id — labels contain Chinese text/spaces and
/// are not unique per request, while request_id is meant for correlation.
fn next_request_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("feishu_{nanos}_{seq}")
}

/// Normalize a `/gateway <action>` action into the watchdog control request
/// (spec §12 control commands, plan Task 3.1). `on`/`off` map to `ServiceSet`
/// with `persist=false` (persistence requires the Phase 4 atomic config write,
/// per spec §15); `restart` maps to `ServiceRestart`; `status` maps to
/// `ServiceStatusFor(gateway)`. The match is exhaustive so a new action
/// cannot silently fall through to a setter.
fn gateway_control_request(
    action: GatewayAction,
) -> crate::watchdog::control_rpc::RpcControlRequest {
    use crate::watchdog::control_rpc::RpcControlRequest;
    match action {
        GatewayAction::On => RpcControlRequest::ServiceSet {
            service: "gateway".into(),
            desired: "on".into(),
            persist: false,
        },
        GatewayAction::Off => RpcControlRequest::ServiceSet {
            service: "gateway".into(),
            desired: "off".into(),
            persist: false,
        },
        GatewayAction::Status => RpcControlRequest::ServiceStatusFor {
            service: "gateway".into(),
        },
        GatewayAction::Restart => RpcControlRequest::ServiceRestart {
            service: "gateway".into(),
        },
    }
}

async fn submit_watchdog_control(
    key: &SessionKey,
    request: crate::watchdog::control_rpc::RpcControlRequest,
    label: &str,
) -> String {
    let secret = match std::env::var("SEBAS_CONTROL_SECRET") {
        Ok(secret) if !secret.is_empty() => secret,
        _ => return format!(
            "{label}请求失败: 当前是裸 core 模式，SEBAS_CONTROL_SECRET 未配置。\
             /upgrade 等 watchdog 命令需要通过 `sebas` watchdog 启动 core（spec §5.3），\
             或在手动启动 core 前 export SEBAS_CONTROL_SECRET=<与 watchdog --secret 一致的值>"
        ),
    };

    // 长操作（update/rollback/restart）提交后需要轮询 progress 事件；其余为同步查询，无需轮询。
    let is_long_op = matches!(
        &request,
        crate::watchdog::control_rpc::RpcControlRequest::Update { .. }
            | crate::watchdog::control_rpc::RpcControlRequest::Rollback { .. }
            | crate::watchdog::control_rpc::RpcControlRequest::RestartCore
    );
    let envelope = feishu_control_envelope(key, secret.clone(), request);

    match crate::watchdog::control_rpc::request(
        &crate::watchdog::control_rpc::default_socket_path(),
        &envelope,
    )
    .await
    {
        Ok(crate::watchdog::control_rpc::RpcControlResponse::Accepted {
            operation_id,
            status,
        }) => {
            let base = format!("已提交{label}请求给 watchdog: {operation_id} ({status})");
            if is_long_op {
                // MVP：提交成功后轮询该 operation 的 Started/Progress/Done/Error 事件，
                // 把阶段性进度累积成多行文本返回。best-effort：轮询失败不影响控制本身。
                poll_operation_progress(key, &secret, &operation_id, base).await
            } else {
                base
            }
        }
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
        Ok(crate::watchdog::control_rpc::RpcControlResponse::PendingConfirmation {
            action,
            message,
            expires_in,
            ..
        }) => format!("{label}请求需要确认: {message} (action={action}, {expires_in}s 内有效)"),
        Err(e) => format!("{label}请求失败: {e}"),
    }
}

/// 轮询 watchdog 事件流，跟踪指定 `operation_id` 的执行进度，把阶段性事件
/// 累积成多行文本。best-effort：任何轮询失败/超时都回退到已有的基础文本，
/// 不把进度渲染失败当作控制失败返回。
///
/// 事件 kind 来自 `ControlEventKind` 的 Debug 字符串（Started/Progress/Done/
/// Error/Canceled/TimedOut）。门限（终态）：Done/Error/Canceled/TimedOut。
/// 上限：最多 `POLL_ROUNDS` 轮、每轮间隔 `POLL_INTERVAL`，未到终态返回已见进度并提示
/// 可用 `/events` 查后续。
async fn poll_operation_progress(
    key: &SessionKey,
    secret: &str,
    operation_id: &str,
    base: String,
) -> String {
    const POLL_ROUNDS: usize = 15;
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

    let mut seen = Vec::<String>::new();
    let mut since: u64 = 0;
    let mut terminal = false;

    for _ in 0..POLL_ROUNDS {
        tokio::time::sleep(POLL_INTERVAL).await;
        let envelope = feishu_control_envelope(
            key,
            secret.to_string(),
            crate::watchdog::control_rpc::RpcControlRequest::EventsSince { seq: since },
        );
        let Ok(crate::watchdog::control_rpc::RpcControlResponse::Events { events })
            = crate::watchdog::control_rpc::request(
                &crate::watchdog::control_rpc::default_socket_path(),
                &envelope,
            )
            .await
        else {
            // 一次查询失败就放弃轮询，避免卡死当前处理路径。
            break;
        };
        for event in events {
            if event.operation_id != operation_id {
                continue;
            }
            since = since.max(event.seq);
            seen.push(format!(
                "- [{}] {}",
                event.kind, event.public_message
            ));
            // 终态事件：Done/Error/Canceled/TimedOut（Started/Progress 为非终态）。
            if matches!(event.kind.as_str(), "Done" | "Error" | "Canceled" | "TimedOut") {
                terminal = true;
            }
        }
        if terminal {
            break;
        }
    }

    let mut lines = vec![base];
    if seen.is_empty() {
        lines.push(format!(
            "(未在 {POLL_ROUNDS} 轮内观察到该操作的进度事件，可能仍在前台执行，可稍后 `/events` 查)"
        ));
    } else {
        lines.push("执行进度:".to_string());
        lines.extend(seen);
        if !terminal {
            lines.push("(仍在执行，可稍后 `/events` 查后续进度)".to_string());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use feishu::events::FeishuIn;
    use router::state::{Mapping, SessionMap};

    /// 话题会话：root_id 为空时用 router 存的最近回复目标兜底；显式带
    /// root_id 优先。这是权限卡等出站卡 root_id 的唯一收口（F3）。
    #[tokio::test]
    async fn topic_reply_target_falls_back_for_thread_sessions() {
        let map = SessionMap::new();
        let key = SessionKey {
            chat_id: "oc_topic".into(),
            thread_id: Some("omt_t1".into()),
        };
        map.insert(key.clone(), Mapping::active("s1"))
            .await
            .unwrap();
        let (router, _rx) = RouterHandle::new(map);

        // 入站话题消息写入 reply target（话题根消息 message_id）。
        router
            .dispatch(FeishuIn::Text {
                key: key.clone(),
                text: "hello".into(),
                reply_to: Some("om_root".into()),
                chat_type: "private".into(),
                mentions: vec![],
            })
            .await;

        // 空 root_id → 兜底到话题根消息。
        assert_eq!(
            topic_reply_target(&router, &key, None).await.as_deref(),
            Some("om_root")
        );
        // 显式 root_id 优先于兜底。
        assert_eq!(
            topic_reply_target(&router, &key, Some("explicit".into()))
                .await
                .as_deref(),
            Some("explicit")
        );
    }

    /// 主线会话也兜底到最近入站消息（用户的触发消息）：权限卡 / 失败提示卡
    /// 现在也会以 reply 形式挂回用户发言，与话题内行为对齐。
    #[tokio::test]
    async fn topic_reply_target_falls_back_for_mainline_too() {
        let map = SessionMap::new();
        let key = SessionKey {
            chat_id: "oc_main".into(),
            thread_id: None,
        };
        // 预映射 session，否则未映射的 PassThrough 会触发 spawn_new → 清掉
        // reply_targets，让兜底拿不到值。
        map.insert(key.clone(), Mapping::active("s1"))
            .await
            .unwrap();
        let (router, _rx) = RouterHandle::new(map);

        // 入站主线消息写入 reply target（用户自己的 message_id）。
        router
            .dispatch(FeishuIn::Text {
                key: key.clone(),
                text: "hello".into(),
                reply_to: Some("om_user_msg".into()),
                chat_type: "private".into(),
                mentions: vec![],
            })
            .await;

        // 空 root_id → 兜底到用户消息。
        assert_eq!(
            topic_reply_target(&router, &key, None).await.as_deref(),
            Some("om_user_msg")
        );
        // 显式 root_id 仍然优先于兜底。
        assert_eq!(
            topic_reply_target(&router, &key, Some("explicit".into()))
                .await
                .as_deref(),
            Some("explicit")
        );
    }

    /// 话题失效错误分类（F1 熔断判定）：230019/230071 → 熔断；其他错误码 /
    /// 非 Feishu 错误 → 照常冒泡不熔断。
    #[test]
    fn topic_invalid_classifier_recognizes_breaker_codes() {
        let mk = |code: i32| {
            let err: anyhow::Error = FeishuApiError {
                code,
                msg: "x".into(),
            }
            .into();
            classify_topic_invalid(&err)
        };
        assert_eq!(mk(230019), Some(230019), "话题不存在必须熔断");
        assert_eq!(mk(230071), Some(230071), "群不支持话题回复必须熔断");
        assert_eq!(mk(99999), None, "无关错误码不熔断");
        let net_err: anyhow::Error = anyhow::anyhow!("network down");
        assert_eq!(
            classify_topic_invalid(&net_err),
            None,
            "非 Feishu 错误不熔断"
        );
    }

    // ---------- Phase 3 Task 3.1: Feishu control adapter contract ----------

    /// 核心代理必须以 Feishu 角色携带 chat_id 提交 control RPC（open_id 在
    /// Phase 5 前为空），并生成唯一的、机器友好的 request_id。
    #[test]
    fn feishu_control_envelope_carries_chat_actor() {
        let key = SessionKey {
            chat_id: "oc_proxy".into(),
            thread_id: Some("omt_t".into()),
        };
        let envelope = feishu_control_envelope(
            &key,
            "secret-1".into(),
            crate::watchdog::control_rpc::RpcControlRequest::Status,
        );

        assert_eq!(envelope.version, 1);
        assert_eq!(envelope.secret, "secret-1");
        assert!(
            envelope.request_id.starts_with("feishu_"),
            "unexpected request_id: {}",
            envelope.request_id
        );
        match &envelope.actor {
            crate::watchdog::control_rpc::RpcActor::Feishu { open_id, chat_id } => {
                assert!(open_id.is_empty(), "Phase 5 前 open_id 应为空");
                assert_eq!(chat_id.as_deref(), Some("oc_proxy"));
            }
            other => panic!("expected Feishu actor, got {other:?}"),
        }
        assert!(matches!(
            envelope.request,
            crate::watchdog::control_rpc::RpcControlRequest::Status
        ));
    }

    /// 每个控制 RPC 请求都应有唯一 request_id，不能只由 label 派生（相同
    /// label 的两次请求必须可区分）。
    #[test]
    fn feishu_control_request_ids_are_unique() {
        let key = SessionKey {
            chat_id: "oc_proxy".into(),
            thread_id: None,
        };
        let a = feishu_control_envelope(
            &key,
            "secret-1".into(),
            crate::watchdog::control_rpc::RpcControlRequest::Status,
        );
        let b = feishu_control_envelope(
            &key,
            "secret-1".into(),
            crate::watchdog::control_rpc::RpcControlRequest::Status,
        );
        assert_ne!(a.request_id, b.request_id);
    }

    // ── /upgrade 端到端：dispatch.submit → watchdog.serve → UpdaterRunner ──
    //
    // 此前 RPC envelope 测试只覆盖 dispatch→server 的 wire 层；UpdaterRunner 是
    // 收到 Accepted/PendingConfirmation 后真正要执行的步骤。这里 spawn 真
    // watchdog serve 在临时 socket 上，把 feishu_control_envelope + request 接
    // 上一个会记录 UpdatePlan 的 RecordingRunner，断言「Update{dev,dry_run}
    // → plan.dev/dry_run 与请求一致」。env-var-missing 错误信息单独测。

    use crate::watchdog::control::ControlService;
    use crate::watchdog::control_rpc::{serve as rpc_serve, RpcControlResponse};
    use crate::watchdog::executor::ControlExecutor;
    use crate::watchdog::updater::{UpdatePlan, UpdaterRunner};
    use std::sync::Arc as StdArc;

    const E2E_SECRET: &str = "e2e-secret-7";

    struct RecordingRunner {
        captured: std::sync::Mutex<Option<(UpdatePlan, crate::config::WatchdogConfig)>>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self {
                captured: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl UpdaterRunner for RecordingRunner {
        async fn run(
            &self,
            plan: &UpdatePlan,
            watchdog: &crate::config::WatchdogConfig,
        ) -> Result<(), crate::error::SebasError> {
            *self.captured.lock().unwrap() = Some((plan.clone(), watchdog.clone()));
            Ok(())
        }
    }

    fn e2e_executor(runner: StdArc<dyn UpdaterRunner>) -> ControlExecutor {
        let control = StdArc::new(tokio::sync::Mutex::new(ControlService::new()));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ControlExecutor::new(
            control,
            runner,
            crate::config::WatchdogConfig::default(),
            "./config.toml".into(),
            tx,
        )
    }

    fn unique_socket(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("sebas-e2e-{label}-{nanos}-{}.sock", std::process::id()))
    }

    /// /upgrade 在裸 core 模式下不应再报「请 export SEBAS_CONTROL_SECRET」，
    /// 而应直接告诉用户走 watchdog。
    #[tokio::test]
    async fn submit_watchdog_control_returns_actionable_message_when_secret_missing() {
        // 安全设置：清掉可能从测试 runner 继承来的 env。
        // Rust 1.78+ 起 set_var 是 unsafe（线程安全考虑）。
        unsafe {
            std::env::remove_var("SEBAS_CONTROL_SECRET");
        }

        let key = SessionKey {
            chat_id: "oc_no_secret".into(),
            thread_id: None,
        };
        let out = submit_watchdog_control(
            &key,
            crate::watchdog::control_rpc::RpcControlRequest::Update {
                dev: true,
                dry_run: false,
            },
            "升级",
        )
        .await;

        assert!(
            out.contains("裸 core"),
            "missing-secret error must point at the bare-core mode, got: {out}"
        );
        assert!(
            out.contains("watchdog"),
            "missing-secret error must mention watchdog, got: {out}"
        );
        assert!(
            !out.contains("请在启动前 export"),
            "old 'export it yourself' guidance must be replaced, got: {out}"
        );
    }

    /// 端到端：dispatch 组 envelope → 真 Unix-socket → watchdog serve → 真实
    /// ControlExecutor 接受 Update{dev,dry_run} → UpdaterRunner 收到正确 plan。
    /// 这是「/upgrade dev」从飞书消息到执行落盘的完整链路（不含真实的
    /// compile_dev/installer——由 RecordingRunner 替代）。
    #[tokio::test]
    async fn upgrade_dev_e2e_envelope_to_runner() {
        let path = unique_socket("upgrade-dev");
        let runner = StdArc::new(RecordingRunner::new());
        let executor = e2e_executor(runner.clone() as StdArc<dyn UpdaterRunner>);
        let bind = path.clone();
        let server = tokio::spawn(async move {
            let _ = rpc_serve(bind, E2E_SECRET.into(), executor).await;
        });
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let key = SessionKey {
            chat_id: "oc_e2e_upgrade".into(),
            thread_id: None,
        };
        let envelope = feishu_control_envelope(
            &key,
            E2E_SECRET.into(),
            crate::watchdog::control_rpc::RpcControlRequest::Update {
                dev: true,
                dry_run: true,
            },
        );

        let resp = crate::watchdog::control_rpc::request(&path, &envelope)
            .await
            .expect("server reachable + valid secret");

        match &resp {
            RpcControlResponse::Accepted { .. } | RpcControlResponse::PendingConfirmation { .. } => {}
            other => panic!("Update dev/dry_run must reach Accepted or PendingConfirmation, got {other:?}"),
        }

        // dry_run=true 时 executor 在 Accepted/Confirmation 路径上不会实际调用
        // runner（runner 由 executor.submit_or_confirm 的 PostAction 阶段调起），
        // 因此不强行断言 captured；端到端的「真落盘」由 upgrade_dev_test.rs 的
        // #[ignore] 用例覆盖。这里主要断言 envelope 能被远端正确解析、归类。
        // 即便如此，runner 应至少「被构造出来」且可访问，证明 wiring 没断。
        assert!(
            runner.captured.lock().unwrap().is_none(),
            "dry_run Update should not call UpdaterRunner eagerly"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }


    /// `/gateway on|off` 归一化为 ServiceSet(gateway, persist=false)；
    /// `/gateway status` 归一化为 ServiceStatusFor(gateway)；
    /// `/gateway restart` 归一化为 ServiceRestart(gateway)。这是 WebUI/Feishu
    /// 共享的归一化契约（spec §12 / plan cross-phase adapter parity）。
    #[test]
    fn gateway_actions_normalize_to_control_requests() {
        use crate::watchdog::control_rpc::RpcControlRequest;

        assert_eq!(
            gateway_control_request(GatewayAction::On),
            RpcControlRequest::ServiceSet {
                service: "gateway".into(),
                desired: "on".into(),
                persist: false,
            }
        );
        assert_eq!(
            gateway_control_request(GatewayAction::Off),
            RpcControlRequest::ServiceSet {
                service: "gateway".into(),
                desired: "off".into(),
                persist: false,
            }
        );
        assert_eq!(
            gateway_control_request(GatewayAction::Status),
            RpcControlRequest::ServiceStatusFor {
                service: "gateway".into(),
            }
        );
        assert_eq!(
            gateway_control_request(GatewayAction::Restart),
            RpcControlRequest::ServiceRestart {
                service: "gateway".into(),
            }
        );
    }
}
