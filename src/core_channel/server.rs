//! Core session channel server (openspec/changes/add-core-session-channel,
//! tasks 5.1–5.8): a Unix-socket NDJSON server inside the core process.
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
};
use crate::error::{Result, SebasError};
use sebas_channels::ChannelKey;
use sebas_router::{RouterHandle, SessionEvent};
use sebas_webui::session_backend::SessionRejection;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
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

/// Bind the Unix listener at `path` with mode 0600, reclaiming a stale socket
/// file (task 5.1). A socket file that still accepts connections means a live
/// server — that's an error, not a stale file.
pub fn bind_channel_socket(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
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
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

/// Serve the core session channel until the process exits. `secret` is the
/// value of `SEBAS_CORE_SECRET` injected by the watchdog (empty disables the
/// secret check only for peers that send an empty secret — the uid check
/// still applies).
pub async fn serve(
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
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(?e, "core channel accept failed");
                        continue;
                    }
                };
                let router = router.clone();
                let secret = secret.clone();
                let mut close_rx = close_rx.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        r = handle_connection(stream, router, secret) => {
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
fn peer_uid_ok(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut ucred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    } == 0;
    ok && ucred.uid == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn peer_uid_ok(_stream: &UnixStream) -> bool {
    true
}

/// Read the handshake line and verify the secret (task 5.3). Absent line,
/// unparseable line, empty-vs-required, or wrong secret → None (caller
/// closes without answering).
async fn read_handshake(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
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
    stream: UnixStream,
    router: RouterHandle,
    secret: String,
) -> Result<()> {
    if !peer_uid_ok(&stream) {
        // 5.2: reject before reading anything.
        warn!("core channel: peer uid mismatch; closing");
        return Ok(());
    }

    let (reader, mut writer) = stream.into_split();
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
                // 5.4: stream connection — snapshot first, then events.
                return serve_subscription(router, writer).await;
            }
            other => {
                let resp = dispatch(&router, other).await;
                write_response(&mut writer, &resp).await?;
            }
        }
    }
}

async fn write_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
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
async fn serve_subscription(
    router: RouterHandle,
    mut writer: tokio::net::unix::OwnedWriteHalf,
) -> Result<()> {
    const FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let mut events = router.subscribe_session_events();
    let snapshot = router.session_info_snapshot().await;

    // Frame 1: the snapshot.
    let frame = SessionStreamFrame::Snapshot { sessions: snapshot };
    write_frame(&mut writer, &frame).await?;

    let mut pending: Vec<SessionEvent> = Vec::new();
    loop {
        match events.recv().await {
            Ok(event) => pending.push(event),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // 5.8: a lagging subscriber is dropped, not gap-filled.
                warn!("core channel: subscriber lagged; dropping connection");
                return Ok(());
            }
            Err(broadcast::error::RecvError::Closed) => {
                return Ok(()); // router gone (core shutting down)
            }
        }
        // Bounded flush: a live local reader drains in microseconds. A
        // stalled reader leaves the socket buffer full → timeout → drop.
        let flush = async {
            for ev in pending.drain(..) {
                write_frame(&mut writer, &SessionStreamFrame::Event { event: ev }).await?;
            }
            writer.flush().await.map_err(SebasError::from)
        };
        if tokio::time::timeout(FLUSH_TIMEOUT, flush).await.is_err() {
            warn!("core channel: subscriber stalled on write; dropping connection");
            return Ok(());
        }
    }
}

async fn write_frame(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &SessionStreamFrame,
) -> Result<()> {
    let json = serde_json::to_string(frame)
        .map_err(|e| SebasError::Upgrade(format!("core channel serialize failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// Dispatch one non-subscribe request against the router.
async fn dispatch(router: &RouterHandle, req: CoreChannelRequest) -> CoreChannelResponse {
    match req {
        CoreChannelRequest::Snapshot => {
            CoreChannelResponse::Snapshot {
                sessions: router.session_info_snapshot().await,
            }
        }
        CoreChannelRequest::Spawn {
            prompt,
            project_dir,
            model,
        } => match &project_dir {
            Some(dir) => {
                // 5.5: canonicalize + stat BEFORE any spawn; no existence
                // disclosure in the rejection message.
                if !usable_project_dir(dir) {
                    CoreChannelResponse::Rejected {
                        rejection: SessionRejection::UnusableProjectDir,
                    }
                } else {
                    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
                    let key = router
                        .web_spawn(prompt, Some(canonical.display().to_string()), None, model)
                        .await;
                    CoreChannelResponse::Spawned { key }
                }
            }
            None => {
                let key = router.web_spawn(prompt, None, None, model).await;
                CoreChannelResponse::Spawned { key }
            }
        },
        CoreChannelRequest::SetSessionModel { key, model_id } => {
            // 中程切换模型：解析路由 session_id 后经 Out::SendAcp 送达 SetModel。
            let Some(sid) = router.map.get(&key).await.and_then(|m| m.session_id().map(str::to_owned))
            else {
                return CoreChannelResponse::Rejected {
                    rejection: SessionRejection::UnknownSession {
                        key: key_str(&key),
                    },
                };
            };
            router
                .emit(sebas_router::Out::SendAcp {
                    session_id: sid.clone(),
                    cmd: sebas_acp::AcpCommand::SetModel {
                        session_id: sid,
                        model_id,
                    },
                })
                .await;
            CoreChannelResponse::Ok
        }
        CoreChannelRequest::Message { key, message } => {
            // 5.6: unknown key → typed rejection, nothing mutated.
            if !router.session_exists(&key).await {
                return CoreChannelResponse::Rejected {
                    rejection: SessionRejection::UnknownSession {
                        key: key_str(&key),
                    },
                };
            }
            router.web_send_message(key, message).await;
            CoreChannelResponse::Ok
        }
        CoreChannelRequest::Close { key } => {
            if !router.session_exists(&key).await {
                return CoreChannelResponse::Rejected {
                    rejection: SessionRejection::UnknownSession {
                        key: key_str(&key),
                    },
                };
            }
            router.web_close_session(key).await;
            CoreChannelResponse::Ok
        }
        CoreChannelRequest::Turns { key, from } => match router.session_turns(&key, from).await {
            Some(entries) => CoreChannelResponse::Turns { entries },
            None => CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnknownSession {
                    key: key_str(&key),
                },
            },
        },
        CoreChannelRequest::SetFocus { key } => {
            router.web_set_active(key).await;
            CoreChannelResponse::Ok
        }
        CoreChannelRequest::Focused => CoreChannelResponse::Focused {
            key: router.active_session_snapshot().await,
        },
        // Handled by the connection loop before dispatch; unreachable here.
        CoreChannelRequest::Subscribe => CoreChannelResponse::Ok,
    }
}

fn key_str(key: &ChannelKey) -> String {
    serde_json::to_string(key).unwrap_or_default()
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
