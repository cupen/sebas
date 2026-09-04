//! The generic ACP driver: spawns a native Agent-Client-Protocol agent
//! (`gemini --acp`, `codex-acp`, …) and drives it through
//! `agent-client-protocol` v1, emitting the crate-level `AcpEvent` vocabulary.
//!
//! `AcpAgent` owns the child process (dropping the connection terminates its
//! process group), so cancel = dropping the run loop, which the manager's
//! wrapper awaits.

mod codec;

use crate::agent_driver::{AgentDriver, DriverConfig, DriverError, DriverHandle};
use crate::session::{AcpCommand, AcpEvent, AcpModelInfo, Decision};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ClientCapabilities, ClientSessionCapabilities,
    ContentBlock, InitializeRequest, LoadSessionRequest, NewSessionRequest, PermissionOption,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigKind,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelect,
    SessionConfigSelectOptions, SessionConfigValueId, SessionNotification,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, TextContent,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo};
use std::collections::HashMap;
use tokio::sync::oneshot;

pub struct AcpDriver;

#[async_trait::async_trait]
impl AgentDriver for AcpDriver {
    async fn spawn(&self, cfg: DriverConfig) -> Result<DriverHandle, DriverError> {
        let DriverConfig {
            kind_slug,
            command,
            work_dir,
            extra_env,
            session_id,
            load_session_id,
            resume,
            startup_timeout,
            evt_tx,
            mut cmd_rx,
            mut cancel_rx,
            pending_perms,
            terminal_sent,
        } = cfg;

        // Resume is implemented below via ACP `session/load` (with an honest
        // fresh-session fallback on rejection); nothing to do up front.

        // Build the transport (spawns the subprocess; drop = process-group
        // termination per the SDK's AcpAgent contract).
        let mut argv = command.into_iter();
        let exe = argv
            .next()
            .ok_or_else(|| DriverError::NotFound("empty command".to_string()))?;
        let args: Vec<String> = argv.collect();
        let mut agent_cfg = AcpAgentConfig::new(exe).args(args);
        for (k, v) in extra_env {
            agent_cfg = agent_cfg.env(k, v);
        }
        let agent = AcpAgent::new(agent_cfg);

        let routing_id = session_id.clone();
        let cwd = work_dir.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".to_string())
        });

        // Handshake completion: initialize + load/new happen inside `run`
        // (the connect closure owns the connection), so the definitive
        // routing id, `resumed` flag, real ACP session id and the session's
        // model selection surface are only known after the run loop starts.
        // The manager awaits this via `DriverHandle::handshake`.
        let (handshake_tx, handshake_rx) =
            oneshot::channel::<(String, bool, Option<String>, Option<AcpModelInfo>)>();

        // The current routing id, shared with the notification/permission
        // hooks (captured before the handshake resolves the final id — a
        // resume-rejection fallback swaps in a fresh id the hooks must use).
        let current_id = std::sync::Arc::new(std::sync::Mutex::new(routing_id.clone()));

        let run = async move {
            // Keep a second handle for the post-connect terminal-error path;
            // the connect closure below moves the original.
            let current_id_outer = current_id.clone();
            let result = Client.builder()
                .on_receive_notification(
                    {
                        let evt_tx = evt_tx.clone();
                        let current_id = current_id.clone();
                        let mut tool_names: HashMap<String, String> = HashMap::new();
                        async move |notification: SessionNotification, _cx| {
                            let sid = current_id.lock().unwrap().clone();
                            for evt in codec::translate_notification(
                                &sid,
                                &mut tool_names,
                                &notification,
                            ) {
                                if evt_tx.send(evt).await.is_err() {
                                    break;
                                }
                            }
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    {
                        let evt_tx = evt_tx.clone();
                        let current_id = current_id.clone();
                        let kind_slug = kind_slug.clone();
                        let pending_perms = pending_perms.clone();
                        async move |request: RequestPermissionRequest, responder, _cx| {
                            let sid = current_id.lock().unwrap().clone();
                            let raw_id = request.tool_call.tool_call_id.to_string();
                            let request_id = format!("{kind_slug}:{raw_id}");
                            let tool_name = request
                                .tool_call
                                .fields
                                .title
                                .clone()
                                .unwrap_or_else(|| "tool".to_string());
                            let args = request
                                .tool_call
                                .fields
                                .raw_input
                                .clone()
                                .unwrap_or(serde_json::Value::Null);
                            let (tx, rx) = oneshot::channel();
                            pending_perms.lock().await.insert(request_id.clone(), tx);
                            let _ = evt_tx
                                .send(AcpEvent::PermissionRequest {
                                    session_id: sid,
                                    request_id: request_id.clone(),
                                    tool_name,
                                    args,
                                })
                                .await;
                            // Park until the manager resolves the oneshot; on
                            // drop (no answerer) fail closed.
                            let decision = match rx.await {
                                Ok(d) => d,
                                Err(_) => Decision::Deny,
                            };
                            let response = map_decision(&decision, &request.options);
                            responder.respond(response)?;
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, {
                    // 连接闭包（async move）里要发 ModelChanged / 非 terminal
                    // Error：在 builder 层克隆，避免 move 掉 run 外层用于终端
                    // 错误路径的 evt_tx（与上面各 hook 的 clone 同模式）。
                    let connect_evt_tx = evt_tx.clone();
                    move |cx: ConnectionTo<Agent>| async move {
                            let connect_evt_tx = connect_evt_tx;
                            // 客户端能力：声明支持 session config options
                            // （select 语义），agent 才会在 session/new / load
                            // 响应里带 configOptions（add-acp-model-selection
                            // D1 —— 模型列表的来源是 agent 响应，非硬编码）。
                            let init_response = cx
                                .send_request(
                                    InitializeRequest::new(ProtocolVersion::V1)
                                        .client_capabilities(client_capabilities()),
                                )
                                .block_task()
                                .await?;

                            // Resume through ACP `session/load` when the session was started as a
                            // load: check the agent's advertised capability
                            // first, fall back to a fresh session with a new
                            // routing id on any failure (unknown conversation
                            // or no load support). The fallback mirrors the
                            // Claude driver's resume-rejection semantics
                            // (sebas-dk8.4).
                            let mut resumed = false;
                            // （add-acp-model-selection）会话建立时 agent 经
                            // `configOptions` 暴露的模型选择面；从 `session/new`
                            // / `session/load` 的响应解析，随握手信号一并上抛。
                            // `None` = agent 未暴露模型选项。
                            let mut session_model: Option<AcpModelInfo> = None;
                            // The agent's real ACP session id to report back
                            // so the caller can persist the routing-id ↔
                            // session-id mapping. `session/load` (and the
                            // loaded conversation) carries the id we asked
                            // with; the fresh path takes it from
                            // `session/new`.
                            let acp_session_id = if resume
                                && init_response.agent_capabilities.load_session
                            {
                                // Load by the caller-provided real ACP session
                                // id when one exists (native-ACP agents like
                                // opencode address a conversation by their OWN
                                // id, which differs from sebas's routing uuid);
                                // fall back to the routing id for agents/records
                                // that have no distinct id.
                                let load_target = load_session_id
                                    .clone()
                                    .unwrap_or_else(|| routing_id.clone());
                                tracing::debug!(
                                    kind = %kind_slug,
                                    routing_id = %routing_id,
                                    load_target = %load_target,
                                    "ACP session/load target resolved",
                                );
                                match cx
                                    .send_request(LoadSessionRequest::new(
                                        load_target.clone(),
                                        cwd.clone(),
                                    ))
                                    .block_task()
                                    .await
                                {
                                    Ok(resp) => {
                                        // The agent loaded the conversation;
                                        // the ACP session id IS the loaded
                                        // conversation id, and prompts below
                                        // address it directly. No session/new.
                                        resumed = true;
                                        session_model = extract_model_info(
                                            resp.config_options.as_deref(),
                                        );
                                        Some(load_target)
                                    }
                                    Err(e) => {
                                        // Load refused (conversation gone or
                                        // agent-side error): honest fresh
                                        // start, keep `resumed=false`.
                                        tracing::warn!(
                                            kind = %kind_slug,
                                            session_id = %routing_id,
                                            error = %e,
                                            "ACP session/load rejected; starting a fresh session",
                                        );
                                        None
                                    }
                                }
                            } else {
                                if resume {
                                    tracing::warn!(
                                        kind = %kind_slug,
                                        session_id = %routing_id,
                                        "ACP agent does not advertise load_session; starting a fresh session",
                                    );
                                }
                                None
                            };

                            let acp_session_id = match acp_session_id {
                                Some(loaded) => loaded,
                                None => {
                                    // Fresh path: mint a new routing id and a
                                    // new ACP session. `resumed` stays false
                                    // (initialized above).
                                    let fresh = uuid::Uuid::new_v4().to_string();
                                    *current_id.lock().unwrap() = fresh.clone();
                                    let resp = cx
                                        .send_request(NewSessionRequest::new(cwd.clone()))
                                        .block_task()
                                        .await?;
                                    // 会话建立响应里的 configOptions 是模型
                                    // 选项的来源（D1）：无模型选项 → None
                                    // （webui 不显示下拉、不报错）。
                                    session_model = extract_model_info(
                                        resp.config_options.as_deref(),
                                    );
                                    resp.session_id.to_string()
                                }
                            };

                            // Publish the definitive routing id + resumed flag + real ACP session
                            // id + model surface to the manager; the connection is up only
                            // from here on, so the manager can proceed
                            // concurrently. The ACP session id is always
                            // reported for native-ACP sessions (fresh:
                            // `session/new`'s id; load: the loaded
                            // conversation id) so the caller can persist the
                            // routing-id ↔ session-id mapping.
                            let final_routing = current_id.lock().unwrap().clone();
                            // `session_model` 后续还要用于初始化本地 current model，
                            // 故握手信号里放克隆。
                            let _ = handshake_tx.send((
                                final_routing.clone(),
                                resumed,
                                Some(acp_session_id.clone()),
                                session_model.clone(),
                            ));

                            // 本地记录的 current model（SetModel 成功后更新，
                            // 作为 session/set_config_option 成功与否的本地判定）。
                            let mut current_model: Option<String> =
                                session_model.as_ref().map(|m| m.current.clone());

                            loop {
                        tokio::select! {
                            _ = &mut cancel_rx => break,
                            cmd = cmd_rx.recv() => match cmd {
                                Some(AcpCommand::CreateSession { prompt, .. })
                                | Some(AcpCommand::ContinueSession { prompt, .. }) => {
                                    cx.send_request(PromptRequest::new(
                                        acp_session_id.clone(),
                                        vec![ContentBlock::Text(TextContent::new(prompt))],
                                    ))
                                    .block_task()
                                    .await?;
                                }
                                Some(AcpCommand::SetModel { model_id, .. }) => {
                                    match set_model(
                                        &cx,
                                        acp_session_id.clone(),
                                        &model_id,
                                        current_model.clone(),
                                    )
                                    .await
                                    {
                                        Ok(new_current) => {
                                            current_model = Some(new_current.clone());
                                            let _ = connect_evt_tx
                                                .send(AcpEvent::ModelChanged {
                                                    session_id: final_routing.clone(),
                                                    model_id: new_current,
                                                })
                                                .await;
                                        }
                                        Err(e) => {
                                            // 失败路径：会话仍可用、本地 current
                                            // 不变。显式发 Error（非 terminal），
                                            // 调用方/UI 呈现"模型不可用/无效"。
                                            tracing::warn!(
                                                kind = %kind_slug,
                                                session_id = %routing_id,
                                                model_id,
                                                error = %e,
                                                "ACP session/set_config_option(model) rejected",
                                            );
                                            let _ = connect_evt_tx
                                                .send(AcpEvent::Error {
                                                    session_id: final_routing.clone(),
                                                    message: format!(
                                                        "set model {model_id:?} failed: {e}"
                                                    ),
                                                    terminal: false,
                                                })
                                                .await;
                                        }
                                    }
                                }
                                Some(AcpCommand::Cancel { .. }) => {
                                    let _ = cx
                                        .send_notification(CancelNotification::new(acp_session_id.clone()));
                                }
                                Some(AcpCommand::PermissionReply { .. }) => {
                                    // Replies ride the pending map, not the
                                    // command channel.
                                }
                                None => break,
                            }
                        }
                    }
                    Ok(())
                    }
                })
                .await;

            if let Err(e) = result
                && !terminal_sent.load(std::sync::atomic::Ordering::SeqCst)
            {
                terminal_sent.store(true, std::sync::atomic::Ordering::SeqCst);
                let sid = current_id_outer.lock().unwrap().clone();
                let _ = evt_tx
                    .send(AcpEvent::Error {
                        session_id: sid,
                        message: format!("acp driver error: {e:#}"),
                        terminal: true,
                    })
                    .await;
            }
        };

        let _ = startup_timeout; // the SDK's connect() is driven inside `run`;
                                 // timeout enforcement is a follow-up (R3).

        Ok(DriverHandle {
            session_id,
            // The definitive value rides the handshake channel; this field is
            // ignored by the manager when `handshake` is present.
            resumed: false,
            // Same: the real ACP session id is resolved inside the run loop's
            // connect closure and delivered via the handshake tuple.
            acp_session_id: None,
            // Same: the session's model surface resolves in the connect
            // closure and rides the handshake tuple.
            model: None,
            handshake: Some(handshake_rx),
            run: Box::pin(run),
        })
    }
}

/// 客户端能力：声明对 session config options 的支持（select 语义），让
/// native-ACP agent 在 `session/new`/`session/load` 响应里带上 configOptions
/// （add-acp-model-selection D1——模型列表的来源是 agent 响应，非硬编码）。
fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities::new().session(
        ClientSessionCapabilities::new()
            .config_options(agent_client_protocol::schema::v1::SessionConfigOptionsCapabilities::new()),
    )
}

/// 从会话建立响应（`session/new` / `session/load` / `set_config_option`）的
/// `configOptions` 里提取模型选择面：取 `id=="model"`（或 category==model）
/// 的 select 选项 → current + options 列表。无 model 选项 → `None`（webui
/// 不显示下拉、不报错）。
fn extract_model_info(opts: Option<&[SessionConfigOption]>) -> Option<AcpModelInfo> {
    let Some(opts) = opts else {
        return None;
    };
    let model = opts.iter().find(|o| {
        o.id.0.as_ref() == "model"
            || o
                .category
                .as_ref()
                .is_some_and(|c| matches!(c, SessionConfigOptionCategory::Model))
    })?;
    let SessionConfigKind::Select(SessionConfigSelect {
        current_value,
        options,
        ..
    }) = &model.kind
    else {
        // 仅 select 型模型选项才构成模型选择面；boolean 等其他 kind 跳过。
        return None;
    };
    let current = current_value.0.to_string();
    // options 可为扁平列表或分组列表；取每组 option 的 value，保序去重。
    let mut seen = std::collections::HashSet::new();
    let mut list: Vec<String> = Vec::new();
    let mut push = |value: &SessionConfigValueId| {
        let v = value.0.to_string();
        if seen.insert(v.clone()) {
            list.push(v);
        }
    };
    match options {
        SessionConfigSelectOptions::Ungrouped(items) => {
            for it in items {
                push(&it.value);
            }
        }
        SessionConfigSelectOptions::Grouped(groups) => {
            for g in groups {
                for it in &g.options {
                    push(&it.value);
                }
            }
        }
        // 未来新增的分组形态（non_exhaustive）：忽略，models 列表保持已知项。
        _ => {}
    }
    Some(AcpModelInfo { current, options: list })
}

/// 发标准 `session/set_config_option {configId:"model", value:<model_id>}`。
///
/// - 成功：以 agent 响应里的 `currentValue`（新模型 id）返回，调用方更新
///   本地 current model 并（可选）发 `ModelChanged`。
/// - 失败（RpcError / 无效模型 / agent 无此能力）：返回显式错误，本地
///   current model 不变。
async fn set_model(
    cx: &ConnectionTo<Agent>,
    acp_session_id: String,
    model_id: &str,
    known_current: Option<String>,
) -> anyhow::Result<String> {
    // `set_config_option` 的 value 走 id 语义（select 选项的 value_id）；
    // 请求按真实 ACP 会话 id 寻址（与 acp-session-id-mapping 联动）。
    let resp: SetSessionConfigOptionResponse = cx
        .send_request(SetSessionConfigOptionRequest::new(
            acp_session_id,
            "model",
            model_id,
        ))
        .block_task()
        .await
        .map_err(|e| anyhow::anyhow!("agent 拒绝设置模型（会话仍使用原模型）: {e:#}"))?;
    // 响应里的 configOptions 反映最新的 currentValue —— 用它刷新本地 current；
    // 响应缺失模型选项时回退到本地已知值（agent 未回显也视为成功）。
    let new_current = extract_model_info(Some(&resp.config_options))
        .map(|m| m.current)
        .or(known_current)
        .ok_or_else(|| anyhow::anyhow!("无法确定设置后的模型 id"))?;
    Ok(new_current)
}

/// Map a sebas [`Decision`] onto an ACP permission response by selecting the
/// offered option whose kind matches; fall back to `Cancelled` (deny) when no
/// option matches or none are offered.
fn map_decision(
    decision: &Decision,
    options: &[PermissionOption],
) -> RequestPermissionResponse {
    let wanted = match decision {
        Decision::AllowOnce => PermissionOptionKind::AllowOnce,
        Decision::AllowSession => PermissionOptionKind::AllowAlways,
        Decision::Deny => PermissionOptionKind::RejectOnce,
    };
    if let Some(opt) = options.iter().find(|o| o.kind == wanted) {
        return RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(opt.option_id.clone()),
        ));
    }
    // No exact match: deny is honest (Cancelled), allow falls back to the
    // first offered option rather than silently failing.
    match decision {
        Decision::Deny => {
            RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
        }
        _ => options
            .first()
            .map(|opt| {
                RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                    SelectedPermissionOutcome::new(opt.option_id.clone()),
                ))
            })
            .unwrap_or_else(|| {
                RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
            }),
    }
}
