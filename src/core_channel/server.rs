//! Core session channel server (openspec/changes/add-core-session-channel,
//! tasks 5.1–5.8): a local-IPC NDJSON server inside the core process
//! (Unix domain socket / Windows named pipe via `sebas_ipc`).
//!
//! The core is the single session authority; this server is how every other
//! process observes and drives sessions. Security model (spec):
//! - socket file mode 0600, reclaiming a stale socket file;
//! - peer uid equality via `SO_PEERCRED`, checked **before** any request is
//!   read;
//! - a shared-secret handshake line (`SEBAS_CORE_SECRET`) before any request;
//! - `project_dir` canonicalized and stat'ed before any spawn, with no
//!   existence disclosure in the rejection;
//! - a lagging subscriber is dropped rather than delivered a gap.

use super::protocol::{
    ChannelHandshake, CoreChannelRequest, CoreChannelResponse, SessionStreamFrame,
    StateStreamFrame,
};
use crate::agent_backend::DualSessionBackend;
use crate::error::{Result, SebasError};
use sebas_channels::ChannelKey;
use sebas_ipc::{IpcListener, IpcStream, ReadHalf, WriteHalf};
use sebas_router::RouterHandle;
use sebas_webui::session_backend::{PermissionNotice, SessionBackend, SessionRejection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Default socket path: `$XDG_RUNTIME_DIR/sebas/core.sock`, falling back to a
/// per-uid temp dir (same convention as the control RPC socket).
pub fn default_socket_path() -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("sebas/core.sock");
    }
    let base = std::env::temp_dir().join("sebas");
    #[cfg(unix)]
    if let Some(uid) = unsafe { Some(libc::getuid()) } {
        return base.join(format!("uid{uid}")).join("core.sock");
    }
    base.join("core.sock")
}

/// Resolve the socket path for the given config: `[watchdog.core] channel_path`
/// overrides the default.
pub fn socket_path(cfg: &crate::config::Config) -> PathBuf {
    match cfg.watchdog.core.channel_path.as_deref() {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_socket_path(),
    }
}

/// Bind the IPC listener at `path` with mode 0600 (unix), reclaiming a stale
/// socket file (task 5.1). A socket that still accepts connections means a
/// live server — that's an error, not a stale file.
pub fn bind_channel_socket(path: &Path) -> Result<IpcListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    if path.exists() {
        // Stale-file reclaim: only if nothing answers on it.
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_probe) => {
                return Err(SebasError::Config(format!(
                    "core session channel {} already served by a live process",
                    path.display()
                )));
            }
            Err(_) => {
                // Nobody home — leftover file from an unclean exit.
                std::fs::remove_file(path)?;
            }
        }
    }
    let listener = sebas_ipc::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

/// Serve the core session channel until the process exits. `backend` is the
/// core's composite session seam (acp + native kernel; design D1 of
/// wire-webui-sebas-agent-e2e) — session mutations are applied through it so
/// detached and in-process share one dispatch. `router` backs the router-
/// level state-store domains. `secret` is the value of `SEBAS_CORE_SECRET`
/// injected by the watchdog (empty disables the secret check only for peers
/// that send an empty secret — the uid check still applies).
pub async fn serve(
    backend: Arc<dyn SessionBackend>,
    router: RouterHandle,
    path: PathBuf,
    secret: String,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let listener = bind_channel_socket(&path)?;
    info!(
        path = %path.display(),
        "core session channel listening"
    );

    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    loop {
        tokio::select! {
            accepted = sebas_ipc::accept(&listener) => {
                let stream = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(?e, "core channel accept failed");
                        continue;
                    }
                };
                let backend = backend.clone();
                let router = router.clone();
                let secret = secret.clone();
                let mut close_rx = close_rx.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        r = handle_connection(stream, backend, router, secret) => {
                            if let Err(e) = r {
                                warn!(?e, "core channel connection failed");
                            }
                        }
                        _ = async { while !*close_rx.borrow_and_update() { if close_rx.changed().await.is_err() { std::future::pending::<()>().await; } } } => {
                            // shutdown: connection dropped with the process
                        }
                    }
                });
            }
            _ = wait_shutdown(shutdown.clone()) => {
                break;
            }
        }
    }

    // Graceful shutdown: close the listener, remove the socket file (5.9).
    drop(listener);
    let _ = tokio::fs::remove_file(&path).await;
    let _ = close_tx.send(true);
    info!("core session channel stopped");
    Ok(())
}

async fn wait_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Peer credential check (task 5.2): the connecting uid must equal our own
/// effective uid. Runs before the handshake or any request is read.
#[cfg(unix)]
fn peer_uid_ok(stream: &IpcStream) -> bool {
    use std::os::fd::{AsFd, AsRawFd};
    // unix 上 interprocess 的 Stream 只有 UdSocket 一个变体，模式不可反驳。
    let sebas_ipc::IpcStream::UdSocket(uds) = stream;
    let mut ucred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(
            uds.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    } == 0;
    ok && ucred.uid == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn peer_uid_ok(_stream: &IpcStream) -> bool {
    true
}

/// Read the handshake line and verify the secret (task 5.3). Absent line,
/// unparseable line, empty-vs-required, or wrong secret → None (caller
/// closes without answering).
async fn read_handshake(
    reader: &mut BufReader<ReadHalf>,
    secret: &str,
) -> Option<()> {
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    if line.trim().is_empty() {
        return None;
    }
    let hs: ChannelHandshake = serde_json::from_str(line.trim()).ok()?;
    // Constant-time-ish comparison is overkill for a local same-uid check
    // that the kernel already gated by uid; keep it simple and honest.
    if hs.secret != secret {
        return None;
    }
    Some(())
}

async fn handle_connection(
    stream: IpcStream,
    backend: Arc<dyn SessionBackend>,
    router: RouterHandle,
    secret: String,
) -> Result<()> {
    if !peer_uid_ok(&stream) {
        // 5.2: reject before reading anything.
        warn!("core channel: peer uid mismatch; closing");
        return Ok(());
    }

    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 5.3: secret handshake line before any request. On success the server
    // sends a tiny ack so the client can report "secret rejected" as a
    // distinct cause instead of guessing from an EOF.
    if read_handshake(&mut reader, &secret).await.is_none() {
        warn!("core channel: handshake failed; closing");
        return Ok(());
    }
    writer
        .write_all(b"{\"handshake\":\"ok\"}\n")
        .await
        .map_err(|e| SebasError::Upgrade(format!("core channel ack write failed: {e}")))?;

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(()); // client closed
        }
        let req: CoreChannelRequest = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                let resp = CoreChannelResponse::Rejected {
                    rejection: SessionRejection::Unavailable {
                        cause: format!("invalid request: {e}"),
                    },
                };
                write_response(&mut writer, &resp).await?;
                continue;
            }
        };
        match req {
            CoreChannelRequest::Subscribe => {
                // 5.4: stream connection — snapshot first, then events
                // (plus approval frames as they arise).
                return serve_subscription(backend, writer).await;
            }
            CoreChannelRequest::StateSubscribe => {
                // State subscription: persistent stream — full snapshot first,
                // then merged change notifications (4.2).
                return serve_state_subscription(router, writer).await;
            }
            other => {
                let resp = dispatch(&backend, &router, other).await;
                write_response(&mut writer, &resp).await?;
            }
        }
    }
}

async fn write_response(
    writer: &mut WriteHalf,
    resp: &CoreChannelResponse,
) -> Result<()> {
    let json = serde_json::to_string(resp)
        .map_err(|e| SebasError::Upgrade(format!("core channel serialize failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// 5.4: snapshot-before-events with no gap and no visible duplicate. The
/// receiver subscribes BEFORE the snapshot is taken, so a mutation racing
/// the subscribe is captured by the snapshot (no gap). The events it also
/// produced are full-state updates; applying them after the snapshot is
/// idempotent (no visible duplicate).
///
/// 5.8: a lagging subscriber is dropped rather than delivered a gap. The
/// broadcast receiver is polled every iteration, so a reader that can't
/// keep up either trips `Lagged` (receiver fell >256 events behind while
/// the writer stalled) or the bounded flush times out on a full socket
/// buffer (stalled reader) — either way the connection is closed and the
/// client re-snapshots on reconnect.
///
/// wire-webui-sebas-agent-e2e: the stream is fed by the composite backend
/// (acp + native lifecycle events) and interleaves approval frames
/// (`ApprovalRequested`) from its review-card feed. Approval frames are not
/// replayed on reconnect — a gated call whose request cannot reach any
/// client fails closed at the kernel (spec).
async fn serve_subscription(
    backend: Arc<dyn SessionBackend>,
    mut writer: WriteHalf,
) -> Result<()> {
    const FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let mut events = backend.subscribe();
    let mut approvals = backend.permission_requests();
    let snapshot = backend.snapshot().await;

    // Frame 1: the snapshot.
    let frame = SessionStreamFrame::Snapshot { sessions: snapshot };
    write_frame(&mut writer, &frame).await?;

    let mut pending: Vec<SessionStreamFrame> = Vec::new();
    loop {
        let frame: SessionStreamFrame = tokio::select! {
            ev = events.recv() => match ev {
                Ok(event) => SessionStreamFrame::Event { event },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // 5.8: a lagging subscriber is dropped, not gap-filled.
                    warn!("core channel: subscriber lagged; dropping connection");
                    return Ok(());
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Ok(()); // backend gone (core shutting down)
                }
            },
            approval = recv_approval(&mut approvals) => {
                SessionStreamFrame::ApprovalRequested { notice: approval }
            }
        };
        pending.push(frame);
        // Bounded flush: a live local reader drains in microseconds. A
        // stalled reader leaves the socket buffer full → timeout → drop.
        let flush = async {
            for frame in pending.drain(..) {
                write_frame(&mut writer, &frame).await?;
            }
            writer.flush().await.map_err(SebasError::from)
        };
        if tokio::time::timeout(FLUSH_TIMEOUT, flush).await.is_err() {
            warn!("core channel: subscriber stalled on write; dropping connection");
            return Ok(());
        }
    }
}

/// Next approval notice from the composite feed, if this backend has one.
/// A missing (or closed) feed never resolves — the select arm simply stays
/// pending and session events keep flowing.
async fn recv_approval(
    approvals: &mut Option<broadcast::Receiver<PermissionNotice>>,
) -> PermissionNotice {
    let Some(rx) = approvals.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        match rx.recv().await {
            Ok(notice) => return notice,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => {
                // Feed gone (backend dropped); keep the stream alive.
                return std::future::pending().await;
            }
        }
    }
}

async fn write_frame(
    writer: &mut WriteHalf,
    frame: &SessionStreamFrame,
) -> Result<()> {
    let json = serde_json::to_string(frame)
        .map_err(|e| SebasError::Upgrade(format!("core channel serialize failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// 取单个域的快照 payload。与 `CoreChannelResponse::StateSnapshot` 共用，
/// 也供状态订阅流的全域快照复用。
async fn snapshot_domain(
    router: &RouterHandle,
    domain: &str,
) -> serde_json::Value {
    let Some(engine) = sebas_router::state_store::engine() else {
        return serde_json::json!({"error": "state store 未初始化"});
    };
    match domain {
        "providers" => {
            let state = engine.load_persisted_state().await;
            serde_json::to_value(&state).unwrap_or_default()
        }
        "settings" => match engine.load_settings().await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => serde_json::Value::Null,
            Err(e) => serde_json::json!({"error": e}),
        },
        "sessions" => {
            let sessions = router.session_info_snapshot().await;
            serde_json::to_value(&sessions).unwrap_or_default()
        }
        "projects" => match engine.load_projects().await {
            Ok(projects) => serde_json::json!({ "projects": projects }),
            Err(e) => serde_json::json!({"error": e}),
        },
        other => serde_json::json!({"error": format!("unknown domain: {other}")}),
    }
}

/// 全域快照：providers / settings / projects / sessions。
async fn state_snapshot_all(router: &RouterHandle) -> serde_json::Value {
    let mut domains = serde_json::Map::new();
    for domain in ["providers", "settings", "projects", "sessions"] {
        domains.insert(domain.to_string(), snapshot_domain(router, domain).await);
    }
    serde_json::Value::Object(domains)
}

/// 4.2 状态订阅：先发全域快照，再持续转发变更通知（合并窗口 100ms，
/// 一串提交合并为一帧）。广播无人订阅/关闭时，快照后保持连接等待。
async fn serve_state_subscription(
    router: RouterHandle,
    mut writer: WriteHalf,
) -> Result<()> {
    use sebas_router::state_store::StateChange;
    const MERGE_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);

    // Frame 1: the full snapshot.
    let domains = state_snapshot_all(&router).await;
    let snapshot = StateStreamFrame::Snapshot { domains };
    write_state_frame(&mut writer, &snapshot).await?;

    // 无广播（引擎未初始化通知通道）时挂起等连接关闭。
    let Some(mut changes) = sebas_router::state_store::subscribe_changes() else {
        info!("state subscription: change broadcast 未初始化, 快照后空闲等待");
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        return Ok(());
    };

    // 合并窗口：收到第一帧后等窗口结束，期间收集的 scope 去重后按序输出。
    let mut pending: Vec<String> = Vec::new();
    loop {
        match changes.recv().await {
            Ok(change) => {
                match change {
                    StateChange::Changed { scope } => {
                        if !pending.iter().any(|s| s == &scope) {
                            pending.push(scope);
                        }
                    }
                    StateChange::Reset => {
                        // 全域重置：输出一帧后重发快照。
                        let reset = StateStreamFrame::Changed {
                            scope: "*".into(),
                        };
                        write_state_frame(&mut writer, &reset).await?;
                        let domains = state_snapshot_all(&router).await;
                        write_state_frame(
                            &mut writer,
                            &StateStreamFrame::Snapshot { domains },
                        )
                        .await?;
                        continue;
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // 落后于 64 帧：语义等价于全域变更，重发快照。
                warn!("state subscriber lagged; resending snapshot");
                let domains = state_snapshot_all(&router).await;
                write_state_frame(
                    &mut writer,
                    &StateStreamFrame::Snapshot { domains },
                )
                .await?;
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                return Ok(()); // engine teardown (core shutting down)
            }
        }
        // 合并窗口静默期：收到更多帧就继续收集；100ms 无新帧则结束窗口。
        let window: std::result::Result<
            std::result::Result<(), SebasError>,
            tokio::time::error::Elapsed,
        > = tokio::time::timeout(
            MERGE_WINDOW,
            async {
                loop {
                    match changes.recv().await {
                        Ok(StateChange::Changed { scope }) => {
                            if !pending.iter().any(|s| s == &scope) {
                                pending.push(scope);
                            }
                        }
                        Ok(StateChange::Reset) => {
                            // 窗口内出现 Reset：立即输出已收集帧再按 Reset 处理。
                            flush_pending(&mut writer, &mut pending).await?;
                            let reset = StateStreamFrame::Changed {
                                scope: "*".into(),
                            };
                            write_state_frame(&mut writer, &reset).await?;
                            let domains = state_snapshot_all(&router).await;
                            write_state_frame(
                                &mut writer,
                                &StateStreamFrame::Snapshot { domains },
                            )
                            .await?;
                            return Ok(());
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            warn!("state subscriber lagged; resending snapshot");
                            flush_pending(&mut writer, &mut pending).await?;
                            let domains = state_snapshot_all(&router).await;
                            write_state_frame(
                                &mut writer,
                                &StateStreamFrame::Snapshot { domains },
                            )
                            .await?;
                            return Ok(());
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            flush_pending(&mut writer, &mut pending).await?;
                            return Ok(());
                        }
                    }
                }
            },
        )
        .await;
        match window {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_elapsed) => {}
        }

        flush_pending(&mut writer, &mut pending).await?;
    }
}

async fn flush_pending(
    writer: &mut WriteHalf,
    pending: &mut Vec<String>,
) -> Result<()> {
    for scope in pending.drain(..) {
        write_state_frame(writer, &StateStreamFrame::Changed { scope }).await?;
    }
    Ok(())
}

async fn write_state_frame(
    writer: &mut WriteHalf,
    frame: &StateStreamFrame,
) -> Result<()> {
    let json = serde_json::to_string(frame)
        .map_err(|e| SebasError::Upgrade(format!("core channel serialize failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// Dispatch one non-subscribe request against the composite session backend
/// (design D1 of wire-webui-sebas-agent-e2e). `router` serves the router-
/// level state-store domains and the web-key existence pre-check for Message
/// (the in-process backend would otherwise spawn a new session for an
/// unknown key — the feishu inbound semantic — while the channel spec
/// demands a typed rejection).
async fn dispatch(
    backend: &Arc<dyn SessionBackend>,
    router: &RouterHandle,
    req: CoreChannelRequest,
) -> CoreChannelResponse {
    match req {
        CoreChannelRequest::Snapshot => CoreChannelResponse::Snapshot {
            sessions: backend.snapshot().await,
        },
        CoreChannelRequest::Spawn {
            prompt,
            project_dir,
            model,
            backend: hint,
        } => {
            // 5.5: canonicalize + stat BEFORE any spawn; no existence
            // disclosure in the rejection message.
            if let Some(dir) = &project_dir {
                if !usable_project_dir(dir) {
                    return CoreChannelResponse::Rejected {
                        rejection: SessionRejection::UnusableProjectDir,
                    };
                }
            }
            let project_dir = project_dir.map(|dir| {
                std::fs::canonicalize(&dir)
                    .unwrap_or_else(|_| PathBuf::from(&dir))
                    .display()
                    .to_string()
            });
            match backend
                .spawn_with(prompt, project_dir, hint.as_deref(), model)
                .await
            {
                Ok(key) => CoreChannelResponse::Spawned { key },
                Err(rejection) => CoreChannelResponse::Rejected { rejection },
            }
        }
        CoreChannelRequest::CreatePlaceholder {
            project_dir,
            model,
            backend: hint,
        } => {
            // 0-turn 占位（P2 修复）：只建行、不 spawn 子进程——空 prompt 绝
            // 不上送 agent。project_dir 校验与 Spawn 同款；执行体 hint 随帧
            // 上送、记在 mapping 上，首条消息触发 spawn 时生效
            // （add-composer-agent-binding：composer 建 0-turn 会话是常态
            // 路径，hint 不上线则用户在创建模式选的 agent 被静默丢弃）。
            if let Some(dir) = &project_dir {
                if !usable_project_dir(dir) {
                    return CoreChannelResponse::Rejected {
                        rejection: SessionRejection::UnusableProjectDir,
                    };
                }
            }
            let project_dir = project_dir.map(|dir| {
                std::fs::canonicalize(&dir)
                    .unwrap_or_else(|_| PathBuf::from(&dir))
                    .display()
                    .to_string()
            });
            match backend
                .create_placeholder(project_dir, hint, model)
                .await
            {
                Ok(key) => CoreChannelResponse::Spawned { key },
                Err(rejection) => CoreChannelResponse::Rejected { rejection },
            }
        }
        CoreChannelRequest::SetSessionModel { key, model_id } => {
            // 按执行体分发（wire-webui-sebas-agent-e2e）：native key → 内核，
            // 其余 → ACP（InProcessBackend 解析 session_id 后经 Out::SendAcp）。
            match backend.set_session_model(key, model_id).await {
                Ok(()) => CoreChannelResponse::Ok,
                Err(rejection) => CoreChannelResponse::Rejected { rejection },
            }
        }
        CoreChannelRequest::Message { key, message } => {
            // 5.6: unknown web/feishu key → typed rejection, nothing mutated.
            // Native keys are unknown to the router map — the native backend
            // rejects them itself.
            if !DualSessionBackend::is_native(&key) && !router.session_exists(&key).await {
                return CoreChannelResponse::Rejected {
                    rejection: SessionRejection::UnknownSession {
                        key: key_str(&key),
                    },
                };
            }
            match backend.message(key, message).await {
                Ok(()) => CoreChannelResponse::Ok,
                Err(rejection) => CoreChannelResponse::Rejected { rejection },
            }
        }
        CoreChannelRequest::Close { key } => match backend.close(key).await {
            Ok(()) => CoreChannelResponse::Ok,
            Err(rejection) => CoreChannelResponse::Rejected { rejection },
        },
        CoreChannelRequest::Turns { key, from } => match backend.turns(key, from).await {
            Ok(entries) => CoreChannelResponse::Turns { entries },
            Err(rejection) => CoreChannelResponse::Rejected { rejection },
        },
        CoreChannelRequest::SetFocus { key } => {
            router.web_set_active(key).await;
            CoreChannelResponse::Ok
        }
        CoreChannelRequest::Focused => CoreChannelResponse::Focused {
            key: router.active_session_snapshot().await,
        },
        CoreChannelRequest::ApprovalAnswer {
            request_id,
            decision,
        } => {
            // 审批决定回填（wire-webui-sebas-agent-e2e）：无待决请求 → typed
            // rejection（fail-closed 语义，拒绝而非伪装成功）。
            if backend.answer_permission(&request_id, decision).await {
                CoreChannelResponse::Ok
            } else {
                CoreChannelResponse::Rejected {
                    rejection: SessionRejection::Unavailable {
                        cause: "无待决审批请求（已回答、超时或未知）".into(),
                    },
                }
            }
        }
        // Handled by the connection loop before dispatch; unreachable here.
        CoreChannelRequest::Subscribe => CoreChannelResponse::Ok,
        CoreChannelRequest::StateSnapshot { domain } => {
            match sebas_router::state_store::engine() {
                Some(_) => {
                    let payload = snapshot_domain(router, &domain).await;
                    CoreChannelResponse::StateSnapshot { domain, payload }
                }
                None => CoreChannelResponse::Rejected {
                    rejection: SessionRejection::Unavailable {
                        cause: "state store 未初始化".into(),
                    },
                },
            }
        }
        CoreChannelRequest::StateMutation { domain, payload } => {
            match sebas_router::state_store::engine() {
                Some(engine) => {
                    let result = match domain.as_str() {
                        "settings" => {
                            let value = payload.get("value").cloned().unwrap_or(payload);
                            engine.save_settings(value).await
                        }
                        "projects" => {
                            project_mutation(engine, &payload).await
                        }
                        "providers" => {
                            providers_mutation(engine, &payload).await
                        }
                        "aliases" => {
                            aliases_mutation(engine, &payload).await
                        }
                        other => Err(format!("unknown domain: {other}")),
                    };
                    match result {
                        Ok(()) => CoreChannelResponse::StateMutationOk,
                        Err(e) => CoreChannelResponse::Rejected {
                            rejection: SessionRejection::Unavailable { cause: e },
                        },
                    }
                }
                None => CoreChannelResponse::Rejected {
                    rejection: SessionRejection::Unavailable {
                        cause: "state store 未初始化".into(),
                    },
                },
            }
        }
        CoreChannelRequest::StateSubscribe => CoreChannelResponse::Ok,
    }
}

fn key_str(key: &ChannelKey) -> String {
    serde_json::to_string(key).unwrap_or_default()
}

/// projects 域 mutation 分发：payload 用 `op` 字段区分子操作。
/// - `{"op": "add", "path": "...", "name": "..."}` → 新增（added_at 取当前时间）
/// - `{"op": "remove", "path": "..."}` → 删除（不存在返回错误）
/// - `{"op": "save", "projects": [...]}` → 全量替换
async fn project_mutation(
    engine: &(dyn sebas_router::state_store::StateStoreEngine + Send + Sync),
    payload: &serde_json::Value,
) -> std::result::Result<(), String> {
    let op = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("save");
    match op {
        "add" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "add: 缺少 path 字段".to_string())?;
            let name = payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "add: 缺少 name 字段".to_string())?;
            let added_at = crate::sebas_state::db::unix_now();
            engine.add_project(path, name, added_at).await
        }
        "remove" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "remove: 缺少 path 字段".to_string())?;
            match engine.remove_project(path).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(format!("remove: project '{path}' 不存在")),
                Err(e) => Err(e),
            }
        }
        "save" => {
            let projects = payload
                .get("projects")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            engine.save_projects(projects).await
        }
        other => Err(format!("projects: 未知 op '{other}'")),
    }
}

/// providers 域 mutation 分发（5.3 admin 写路径通道代理）。
/// payload `op` 子操作：
/// - `{"op":"put","name":"...","item":{...}}` → upsert provider
/// - `{"op":"delete","name":"..."}` → 删除 + 写墓碑
/// - `{"op":"save","state":{PersistedState 形状}}` → 全量替换
/// 全部经 RMW（读 → 改 → save_persisted_state），与 router 卡片写路径同语义。
async fn providers_mutation(
    engine: &(dyn sebas_router::state_store::StateStoreEngine + Send + Sync),
    payload: &serde_json::Value,
) -> std::result::Result<(), String> {
    let op = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("save");
    let mut state = engine.load_persisted_state().await;
    match op {
        "put" => {
            let name = payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "put: 缺少 name 字段".to_string())?;
            let item = payload
                .get("item")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .ok_or_else(|| "put: 缺少 item 对象".to_string())?;
            state.providers.insert(name.to_string(), item);
            // 撤销同名墓碑（re-add）。
            state.deleted.retain(|d| d != name);
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "delete" => {
            let name = payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "delete: 缺少 name 字段".to_string())?;
            let existed = state.providers.remove(name).is_some();
            if !state.deleted.iter().any(|d| d == name) {
                state.deleted.push(name.to_string());
            }
            if state.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(name) {
                state.default_selection = None;
            }
            if !existed {
                return Err(format!("provider '{name}' 不存在"));
            }
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "save" => {
            let raw = payload
                .get("state")
                .cloned()
                .ok_or_else(|| "save: 缺少 state 字段".to_string())?;
            let incoming: sebas_router::state_store::PersistedState =
                serde_json::from_value(raw)
                    .map_err(|e| format!("save: 非法 PersistedState: {e}"))?;
            // 保留 mode/default_selection（admin 面不管运行时状态，只写 provider 数据）。
            let mut merged = incoming;
            merged.mode = state.mode.clone();
            merged.default_selection = state.default_selection.clone();
            engine
                .save_persisted_state(merged)
                .await
                .map_err(|e| e.to_string())
        }
        other => Err(format!("providers: 未知 op '{other}'")),
    }
}

/// aliases 域 mutation 分发（5.3 admin 写路径通道代理）。
/// payload `op` 子操作：
/// - `{"op":"put","alias":"...","entry":{"provider":"...","upstream_model":"..."}}`
/// - `{"op":"delete","alias":"..."}`
/// - `{"op":"save","aliases":{alias: entry,...}}` → 全量替换
async fn aliases_mutation(
    engine: &(dyn sebas_router::state_store::StateStoreEngine + Send + Sync),
    payload: &serde_json::Value,
) -> std::result::Result<(), String> {
    let op = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("save");
    let mut state = engine.load_persisted_state().await;
    match op {
        "put" => {
            let alias = payload
                .get("alias")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "put: 缺少 alias 字段".to_string())?;
            let entry: sebas_router::state_store::ModelAliasEntry = serde_json::from_value(
                payload
                    .get("entry")
                    .cloned()
                    .ok_or_else(|| "put: 缺少 entry 对象".to_string())?,
            )
            .map_err(|e| format!("put: entry 非法: {e}"))?;
            state
                .model_aliases
                .insert(alias.to_string(), entry);
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "delete" => {
            let alias = payload
                .get("alias")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "delete: 缺少 alias 字段".to_string())?;
            if state.model_aliases.remove(alias).is_none() {
                return Err(format!("alias '{alias}' 不存在"));
            }
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "save" => {
            let raw = payload
                .get("aliases")
                .cloned()
                .ok_or_else(|| "save: 缺少 aliases 字段".to_string())?;
            let incoming: std::collections::BTreeMap<
                String,
                sebas_router::state_store::ModelAliasEntry,
            > = serde_json::from_value(raw).map_err(|e| format!("save: aliases 非法: {e}"))?;
            state.model_aliases = incoming;
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        other => Err(format!("aliases: 未知 op '{other}'")),
    }
}

/// 5.5: the directory must exist and be a directory. The rejection carries
/// no path detail — callers see "unusable" only.
fn usable_project_dir(dir: &str) -> bool {
    let p = Path::new(dir);
    p.is_dir() && std::fs::canonicalize(p).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 5.1 验收：同一路径连续 bind 两次，第二次成功（stale socket 回收）。
    #[tokio::test]
    async fn binding_twice_in_a_row_reclaims_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        {
            let _l = bind_channel_socket(&path).expect("first bind");
            #[cfg(unix)]
            assert!(path.exists());
            // The listener is alive; a second bind must NOT steal it.
            assert!(bind_channel_socket(&path).is_err(), "live socket must refuse rebind");
        }
        // Listener dropped → socket file is stale → second bind reclaims it.
        let _l2 = bind_channel_socket(&path).expect("stale socket reclaimed");
    }

    // 5.1: mode 0600.
    #[cfg(unix)]
    #[tokio::test]
    async fn socket_file_gets_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let _l = bind_channel_socket(&path).expect("bind");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // 5.5: directory check — non-directory rejected without disclosure.
    #[tokio::test]
    async fn unusable_project_dir_is_detected_without_disclosure() {
        assert!(usable_project_dir(env!("CARGO_MANIFEST_DIR")));
        assert!(!usable_project_dir("/definitely/not/a/dir/sebas-test"));
        // A file is not a directory.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(!usable_project_dir(file.to_str().unwrap()));
    }
}
