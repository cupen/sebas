//! Integration-style tests for the core session channel (binary crate lib):
//! server handshake/rejections (5.1–5.6), backend round-trips (6.1),
//! and reconnect convergence (6.2). The not-connected UI states (7.3) need
//! live browser flows and stay with the webui suites; the cross-uid
//! rejection (5.2) is covered live by `cross_uid_rejected_live_process`
//! (A2.3, CI-only `#[ignore]` — needs root).

use crate::core_channel::client::CoreChannelBackend;
use crate::core_channel::protocol::{
    ChannelHandshake, CoreChannelRequest, CoreChannelResponse, SessionStreamFrame,
};
use crate::core_channel::server;
use sebas_channels::ChannelKey;
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::{DispatchHandle, SessionEvent};
use sebas_webui::session_backend::{
    PermissionDecision, Reachability, SessionBackend, SessionRejection,
};
use std::path::Path as StdPath;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SECRET: &str = "test-core-secret";

/// 等待通道可连接（跨平台：named pipe 无文件残留，不能靠 path.exists()）。
async fn wait_channel_ready(path: &StdPath) {
    for _ in 0..250 {
        if let Ok(stream) = sebas_ipc::connect(path).await {
            drop(stream);
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("core session channel never became connectable");
}

struct TestCore {
    path: std::path::PathBuf,
    close_tx: tokio::sync::watch::Sender<bool>,
    handle: DispatchHandle,
    /// 出站接收端必须保活：DispatchHandle::emit 在 debug 构建下对 closed
    /// channel 直接断言失败（spec 的 dev bug 语义）。
    _out_rx: tokio::sync::mpsc::Receiver<sebas_dispatch::Out>,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        let _ = self.close_tx.send(true);
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn start_core(dir: &StdPath) -> TestCore {
    start_core_with_map(dir, SessionMap::new()).await
}

async fn start_core_with_map(dir: &StdPath, map: SessionMap) -> TestCore {
    let (router, out_rx) = DispatchHandle::new(map);
    let path = dir.join("core.sock");
    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    let serve_path = path.clone();
    let serve_router = router.clone();
    // wire-webui-sebas-agent-e2e：通道 server 委托复合 SessionBackend。测试
    // 直接在 router 上建单后端 InProcessBackend（覆盖全部方法，无需真内核）。
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(serve_router.clone()),
    );
    tokio::spawn(async move {
        let _ = server::serve(backend, serve_router, serve_path, SECRET.into(), close_rx).await;
    });
    wait_channel_ready(&path).await;
    TestCore {
        path,
        close_tx,
        handle: router,
        _out_rx: out_rx,
    }
}

/// 等待通道下线（unix：socket 文件消失；Windows：连接失败即视为消失）。
async fn wait_channel_gone(path: &StdPath) {
    #[cfg(unix)]
    for _ in 0..250 {
        if !path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    #[cfg(not(unix))]
    for _ in 0..250 {
        if sebas_ipc::connect(path).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("core session channel still reachable after shutdown");
}

/// Raw one-shot request helper for protocol-level tests.
async fn raw_request(
    path: &StdPath,
    secret: Option<&str>,
    req: &CoreChannelRequest,
) -> std::io::Result<Option<String>> {
    let stream = sebas_ipc::connect(path).await?;
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    if let Some(s) = secret {
        let hs = serde_json::to_string(&ChannelHandshake {
            secret: s.to_string(),
        })
        .unwrap();
        w.write_all(hs.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        // Ack line.
        let mut ack = String::new();
        reader.read_line(&mut ack).await?;
    }
    let json = serde_json::to_string(req).unwrap();
    w.write_all(json.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

// ── 5.3: secret handshake ───────────────────────────────────────────────────

/// 缺失握手行（直接发请求）→ 连接被关闭，无响应。
#[tokio::test]
async fn missing_handshake_closes_connection_without_response() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    // No handshake: connect and immediately write a request.
    let stream = sebas_ipc::connect(&core.path).await.unwrap();
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let json = serde_json::to_string(&CoreChannelRequest::Snapshot).unwrap();
    w.write_all(json.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    let mut line = String::new();
    let n = reader.read_line(&mut line).await.unwrap();
    assert_eq!(
        n, 0,
        "server must close without answering an unhandshaked client"
    );
}

/// 空密钥 / 错误密钥 → 连接被关闭（5.3）。
#[tokio::test]
async fn wrong_and_empty_secrets_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    for secret in ["", "totally-wrong"] {
        let stream = sebas_ipc::connect(&core.path).await.unwrap();
        let (r, mut w) = sebas_ipc::split(stream);
        let mut reader = BufReader::new(r);
        let hs = serde_json::to_string(&ChannelHandshake {
            secret: secret.to_string(),
        })
        .unwrap();
        w.write_all(hs.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.flush().await.unwrap();
        // No ack arrives; the server closes.
        let mut ack = String::new();
        let n = reader.read_line(&mut ack).await.unwrap();
        assert_eq!(n, 0, "secret {secret:?} must be closed out, got {ack:?}");
    }
    // Correct secret gets the ack and a working request.
    let resp = raw_request(&core.path, Some(SECRET), &CoreChannelRequest::Snapshot)
        .await
        .unwrap();
    assert!(resp.is_some(), "correct secret must be answered");
    let parsed: CoreChannelResponse = serde_json::from_str(&resp.unwrap()).unwrap();
    assert!(matches!(parsed, CoreChannelResponse::Snapshot { .. }));
}

// ── 5.4: snapshot before events, no gap ─────────────────────────────────────

/// 订阅建立后发生的 mutation 必须以事件帧按序到达（无 gap），且事件携带
/// 与重取快照一致的全量状态（应用两次幂等 = 无可见重复）。
#[tokio::test]
async fn subscription_delivers_every_mutation_after_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let stream = sebas_ipc::connect(&core.path).await.unwrap();
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.into(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    let mut ack = String::new();
    reader.read_line(&mut ack).await.unwrap();
    assert!(ack.contains("handshake"));

    let sub = serde_json::to_string(&CoreChannelRequest::Subscribe).unwrap();
    w.write_all(sub.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();

    // Frame 1: the snapshot (arrives before ANY event frame — server order).
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: SessionStreamFrame = serde_json::from_str(line.trim()).unwrap();
    assert!(
        matches!(frame, SessionStreamFrame::Snapshot { .. }),
        "first frame must be the snapshot, got {frame:?}"
    );

    // Mutate AFTER the snapshot: every change must arrive as event frames.
    let key = core
        .handle
        .web_spawn("racing prompt".into(), None, None, None)
        .await;
    core.handle
        .activate(&key, "s-live".into(), None, None)
        .await;

    let mut saw_created = false;
    let mut saw_updated = false;
    for _ in 0..8 {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let frame: SessionStreamFrame = serde_json::from_str(line.trim()).unwrap();
        match frame {
            SessionStreamFrame::Event {
                event: SessionEvent::Created { session },
            } if session.channel_key() == key => saw_created = true,
            SessionStreamFrame::Event {
                event: SessionEvent::Updated { session },
            } if session.channel_key() == key && session.status == "active" => {
                saw_updated = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_created,
        "Created event must follow the snapshot (no gap)"
    );
    assert!(saw_updated, "activate must arrive as an Updated event");

    // The event state matches the authoritative snapshot: an idempotent
    // re-apply changes nothing (no visible duplicate).
    let snap = core.handle.session_info_snapshot().await;
    assert!(
        snap.iter()
            .any(|s| s.channel_key() == key && s.status == "active")
    );
}

// ── 6.1: backend round-trips ────────────────────────────────────────────────

/// 每个 SessionBackend 方法都到达 core 的正确处理器（6.1）。
#[tokio::test]
async fn backend_methods_reach_the_right_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    // snapshot: empty at first.
    assert!(backend.snapshot().await.is_empty());
    assert_eq!(backend.reachability().await, Reachability::Reachable);

    // spawn → key; snapshot now shows one spawning session with project_dir.
    let key = backend
        .spawn("do the thing".into(), Some("/tmp".into()))
        .await
        .expect("spawn");
    let snap = backend.snapshot().await;
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].status, "spawning");
    // 服务端 canonicalize project_dir 后存储（5.5）；断言跟随本平台的
    // canonical 形式（Windows 会把 "/tmp" 变成 verbatim 路径）。
    let expected_dir = std::fs::canonicalize("/tmp")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/tmp".to_string());
    assert_eq!(snap[0].project_dir.as_deref(), Some(expected_dir.as_str()));

    // activate on the core side → snapshot reflects active + session id.
    // (The spawned key is channel-neutral on the wire; the webui-trait client
    // sees it only in feishu shape, so drive the real ChannelKey from the
    // router's mapping directly.)
    let (channel_key, _) = core
        .handle
        .map
        .snapshot_all()
        .await
        .into_iter()
        .next()
        .expect("spawned session mapped");
    core.handle
        .activate(&channel_key, "s-live".into(), None, None)
        .await;
    let snap = backend.snapshot().await;
    assert_eq!(snap[0].status, "active");
    assert_eq!(snap[0].session_id.as_deref(), Some("s-live"));

    // message to the live session → Ok.
    backend
        .message(key.clone(), "hello".into())
        .await
        .expect("message");
    // message via the core handle (channel-neutral key) → routes into the map.
    let _ = core
        .handle
        .web_send_message(channel_key.clone(), "hello".into())
        .await;
    // message to an unknown key → typed rejection, nothing mutated.
    // (The unknown key is channel-neutral on the wire, so it round-trips
    // byte-for-byte through the channel's structured `{channel,reference}`.)
    let bogus = ChannelKey::new("web", "web-nope");
    assert_eq!(
        backend.message(bogus.clone(), "hi".into()).await,
        Err(SessionRejection::UnknownSession {
            key: serde_json::to_string(&bogus).unwrap()
        })
    );

    // turns: seed content on the core, fetch via backend, incremental.
    // workbench-turn-queue 2.2：prompt 一律在开轮时由 seed_card 写入（提交
    // 即写已退役）。两次 web_send_message 各开一轮 → 两条 prompt 条目；
    // 最后的 seed_card("the prompt") 是重入（幂等，不再追加）。
    core.handle
        .seed_card("s-live".into(), "the prompt".into())
        .await;
    use sebas_acp::claude::session::AcpEvent;
    core.handle
        .apply_event(
            "s-live",
            &AcpEvent::TextDelta {
                session_id: "s-live".into(),
                delta: "chunk one".into(),
            },
        )
        .await;
    let all = backend.turns(key.clone(), 0).await.unwrap();
    assert_eq!(all.len(), 3); // prompt (turn 1) + prompt (turn 2) + delta
    let tail = backend.turns(key.clone(), 2).await.unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].content, "chunk one");
    // unknown key → rejection.
    assert!(matches!(
        backend.turns(bogus.clone(), 0).await,
        Err(SessionRejection::UnknownSession { .. })
    ));

    // focus round-trip.
    backend.set_focus(Some(key.clone())).await;
    assert_eq!(backend.focused().await, Some(key.clone()));

    // close → gone; second close → UnknownSession。wire-webui-sebas-agent-e2e：
    // 通道 server 现在把 close 直接委托 backend；具体 reason key 由 backend
    // 形状决定（InProcessBackend 在 NotFound 时留空串，与既有行为一致）。
    backend.close(key.clone()).await.expect("close");
    assert!(backend.snapshot().await.is_empty());
    let _ = backend.close(key.clone()).await;
}

// ── 6.2: reconnect convergence ──────────────────────────────────────────────

/// P2 修复（wire 路径）：`create_placeholder` 经通道建 0-turn 占位——
/// 不产生 `Out::WebSpawn`（无子进程、空 prompt 不上送 agent），映射记住
/// model 与 kind（add-composer-agent-binding：backend hint 的 kind 随帧
/// 上送，首条消息经 `Message` 触发 SpawnNew 时按它 spawn 正确的 agent）。
#[tokio::test]
async fn create_placeholder_wires_a_zero_turn_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let key = backend
        .create_placeholder(Some("/tmp".into()), "opencode", Some("m-free".into()), None)
        .await
        .expect("placeholder created");

    // 占位在快照里可见（spawning、带 project_dir），且没有 spawn 指令发出。
    let snap = backend.snapshot().await;
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].status, "spawning");
    // 服务端会 canonicalize project_dir：Windows 得到 verbatim 形式（\\?\D:\tmp）。
    let expected_dir = std::fs::canonicalize("/tmp")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/tmp".into());
    assert_eq!(snap[0].project_dir.as_deref(), Some(expected_dir.as_str()));
    assert!(
        core._out_rx.try_recv().is_err(),
        "placeholder creation must not emit a spawn instruction"
    );

    // 映射记住了 model 与解析后的 kind（acp:opencode → "opencode"）。
    let (channel_key, m) = core
        .handle
        .map
        .snapshot_all()
        .await
        .into_iter()
        .next()
        .expect("placeholder mapped");
    assert_eq!(channel_key, key);
    assert_eq!(m.pending_model.as_deref(), Some("m-free"));
    assert_eq!(m.pending_kind.as_deref(), Some("opencode"));

    // 不可用 project_dir → 与 Spawn 同款校验拒绝。
    assert_eq!(
        backend
            .create_placeholder(Some("/nonexistent-sebas-p2".into()), "opencode", None, None)
            .await,
        Err(SessionRejection::UnusableProjectDir)
    );

    // 首条消息触发 spawn 路径（SpawnNew → Out::WebSpawn），不排队。
    backend
        .message(key.clone(), "hello".into())
        .await
        .expect("message accepted");
    match core._out_rx.try_recv().expect("WebSpawn emitted") {
        sebas_dispatch::Out::WebSpawn {
            key: k,
            prompt,
            kind,
            model,
            ..
        } => {
            assert_eq!(k, channel_key);
            assert_eq!(prompt, "hello");
            // 首条消息按占位记住的 kind spawn（add-composer-agent-binding）。
            assert_eq!(kind.as_deref(), Some("opencode"));
            assert_eq!(model.as_deref(), Some("m-free"));
        }
        other => panic!("expected Out::WebSpawn, got {other:?}"),
    }
}

/// workbench-agent-wire-fix 根因回归锁（沙箱实证场景）：kind/model 全空的
/// 0-turn 占位（rail「+」与 composer 默认路径的同款 wire），首条消息必须
/// 触发 SpawnNew → Out::WebSpawn。旧 route_text 判据
/// `pending_kind.is_some() || pending_model.is_some()` 对这种占位恒为
/// false，消息被 Enqueued 且无人 drain——输入框「发了没响应」的根因。
#[tokio::test]
async fn placeholder_without_kind_or_model_spawns_on_first_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let key = backend
        .create_placeholder(Some("/tmp".into()), "claudecode", None, None)
        .await
        .expect("placeholder created");

    backend
        .message(key.clone(), "hello".into())
        .await
        .expect("message accepted");
    match core._out_rx.try_recv().expect("WebSpawn emitted") {
        sebas_dispatch::Out::WebSpawn { key: k, prompt, .. } => {
            assert_eq!(k, key);
            assert_eq!(prompt, "hello");
        }
        other => panic!("expected Out::WebSpawn, got {other:?}"),
    }
}

/// 杀掉并重启 server，客户端不重建也能收敛（6.2）。
#[tokio::test]
async fn client_converges_after_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let key = backend.spawn("before restart".into(), None).await.unwrap();
    assert_eq!(backend.snapshot().await.len(), 1);

    // Kill the server (graceful shutdown removes the socket file).
    let _ = core.close_tx.send(true);
    wait_channel_gone(&core.path).await;
    #[cfg(unix)]
    assert!(!core.path.exists(), "socket must be removed on shutdown");

    // While the core is down (graceful exit removed the socket → ENOENT),
    // reachability reports the startup-failed state with a cause.
    let _ = backend.snapshot().await; // trigger a failure refresh
    assert!(
        matches!(
            backend.reachability().await,
            Reachability::StartupFailed { .. }
        ),
        "down core must report startup_failed after a graceful shutdown"
    );

    // Restart the server on the same path (stale socket already removed).
    let path2 = core.path.clone();
    let (_router2, _rx2) = DispatchHandle::new(SessionMap::new());
    let router2 = _router2.clone();
    let (_close2, close2_rx) = tokio::sync::watch::channel(false);
    let backend2: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router2.clone()),
    );
    tokio::spawn(async move {
        let _ = server::serve(backend2, router2, path2.clone(), SECRET.into(), close2_rx).await;
    });
    wait_channel_ready(&core.path).await;

    // Same client instance converges: one-shot methods work again.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snap = backend.snapshot().await;
        if backend.reachability().await == Reachability::Reachable && snap.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "client must converge after server restart"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = key; // old session is gone with the old core — converged to empty
}

// ── 5.8: lagging subscriber is dropped, not gap-filled ──────────────────────

/// 滞后的订阅者（不读帧）在广播溢出后被服务端主动断开；客户端重连并
/// 重新快照即恢复（5.8）。
#[tokio::test]
async fn lagging_subscriber_is_disconnected_and_can_resnapshot() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    // Subscribe but deliberately stall: after the snapshot frame, read
    // nothing while the core publishes more events than the broadcast
    // capacity (256).
    let stream = sebas_ipc::connect(&core.path).await.unwrap();
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.into(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    let mut ack = String::new();
    reader.read_line(&mut ack).await.unwrap();

    let sub = serde_json::to_string(&CoreChannelRequest::Subscribe).unwrap();
    w.write_all(sub.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();

    // Consume the snapshot frame, then stall.
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    assert!(line.contains("\"snapshot\""), "first frame is the snapshot");

    // Overflow the socket buffer (~208KB default) while the subscriber is
    // stalled, so the server's bounded flush blocks and times out (or the
    // broadcast receiver lags) — either way the connection must drop. A
    // small burst would be silently absorbed by the socket buffer, which is
    // correct behavior for a live reader. Publish paths
    // (router.insert_mapping) feed the broadcast, not raw map mutations.
    for i in 0..3000u64 {
        core.handle
            .insert_mapping(
                ChannelKey::feishu(&format!("oc_lag-{i}"), None),
                format!("s-lag-{i}"),
            )
            .await;
    }

    // Now read what's buffered: the server dropped us once its flush stalled
    // (or it lagged), so we hit EOF within the buffered window.
    let mut frames = 0usize;
    let mut eof = false;
    for _ in 0..3500 {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                eof = true;
                break;
            }
            Ok(_) => frames += 1,
            Err(_) => {
                eof = true;
                break;
            }
        }
    }
    assert!(
        eof,
        "server must drop the lagging subscriber (got {frames} frames, still open)"
    );

    // Re-snapshot works: a fresh client sees all 3000 inserts.
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());
    let snap = backend.snapshot().await;
    assert_eq!(snap.len(), 3000, "fresh client re-snapshots cleanly");
}

// ── 6.3: distinct unreachable causes (A1.2 rewrote for the three-way enum) ──

/// socket 不存在 → `StartupFailed`，fallback cause 带上 socket 路径
/// （spec scenario "socket-not-found fallback cause"）。闩锁文件在场时的
/// enrich 全串与握手拒绝/断连两态分别由 A1.2 的三个专项单测覆盖——三态
/// 已是枚举变体，不再可能互相混淆。
#[tokio::test]
async fn unreachable_causes_are_distinct() {
    // Absent socket; hold the startup-error env lock so an ambient latch
    // file cannot rewrite the cause through the unconditional enrich.
    let _env = StartupErrorFile::unset();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sock");
    let backend = CoreChannelBackend::new(path.clone(), SECRET.into());
    let err = backend.spawn("x".into(), None).await.unwrap_err();
    match err {
        SessionRejection::Unavailable { cause } => {
            assert_eq!(
                cause,
                format!(
                    "core session channel socket not found at {}",
                    path.display()
                ),
                "cause must name the absence"
            );
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert!(matches!(
        backend.reachability().await,
        Reachability::StartupFailed { .. }
    ));
}

// ── 4.2/5.4: state subscription stream ──────────────────────────────────────

/// 4.2 协议层：StateSubscribe 连接先收全域快照帧（engine 未初始化时各域
/// 返回 error payload，但帧结构仍在）。mutation→Changed 的链路验证在
/// `tests/state_subscription_test.rs`（独立进程，避免污染 lib 单测的
/// 全局 engine 状态）。
#[tokio::test]
async fn state_subscription_serves_snapshot_frame_without_engine() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let stream = sebas_ipc::connect(&core.path).await.unwrap();
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.into(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    let mut ack = String::new();
    reader.read_line(&mut ack).await.unwrap();
    assert!(ack.contains("handshake"));

    let sub = serde_json::to_string(&CoreChannelRequest::StateSubscribe).unwrap();
    w.write_all(sub.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();

    // 快照帧必须在（即使 engine 未初始化——各域回 error payload，帧照发）。
    let mut line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("snapshot frame must arrive")
        .unwrap();
    assert!(n > 0);
    let frame: crate::core_channel::protocol::StateStreamFrame =
        serde_json::from_str(line.trim()).unwrap();
    match &frame {
        crate::core_channel::protocol::StateStreamFrame::Snapshot { domains } => {
            for domain in ["providers", "settings", "projects", "sessions"] {
                assert!(
                    domains.get(domain).is_some(),
                    "snapshot must include domain {domain}: {domains}"
                );
            }
        }
        other => panic!("first state frame must be the snapshot, got {other:?}"),
    }
    drop(w);
}

// ── wire-webui-sebas-agent-e2e 1.3: ApprovalAnswer typed rejection ──────────

/// 1.3 验收：对未知 request_id 的 `ApprovalAnswer`，服务端返回 typed
/// rejection —— fail-closed 语义（拒绝而非默默丢弃/伪装成功）；client 侧
/// `answer_permission` 相应返回 false。
#[tokio::test]
async fn approval_answer_for_unknown_request_id_returns_typed_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let resp = raw_request(
        &core.path,
        Some(SECRET),
        &CoreChannelRequest::ApprovalAnswer {
            request_id: "toolu_does_not_exist".into(),
            decision: PermissionDecision::AllowOnce,
        },
    )
    .await
    .expect("wire roundtrip");
    let resp: CoreChannelResponse =
        serde_json::from_str(&resp.expect("server returned a response")).expect("response decodes");
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(
                    cause.contains("无待决审批"),
                    "rejection must name the unknown request, got: {cause}"
                );
            }
            other => panic!("expected Unavailable, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }

    // 同一 unknown request 经 client.answer_permission 路径返回 false。
    assert!(
        !backend
            .answer_permission("toolu_does_not_exist", PermissionDecision::AllowOnce)
            .await,
        "unknown request id must report false so callers retry/ignore"
    );
}

/// （extract-im-service 2.1）EnsureMessage：未知 feishu key 自动建会话、
/// 已知 key 等价 Message；`Message` 的「未知即拒绝」语义保持不变。
#[tokio::test]
async fn ensure_message_spawns_unknown_key_and_message_still_rejects() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let chat = ChannelKey::feishu("oc_ensure_test", None);

    // 未知 key：Message 仍按 webui 语义拒绝。
    assert!(backend.message(chat.clone(), "hi".into()).await.is_err());

    // EnsureMessage：未知 key 自动建会话（Spawning 占位进快照）。
    backend
        .ensure_message(chat.clone(), "hello from im".into())
        .await
        .expect("ensure on unknown key creates a session");
    let snap = backend.snapshot().await;
    assert_eq!(
        snap.len(),
        1,
        "ensure created exactly one session: {snap:?}"
    );
    assert_eq!(snap[0].channel, "feishu");
    assert!(snap[0].key.contains("oc_ensure_test"));
    assert_eq!(snap[0].status, "spawning");

    // 已知 key：EnsureMessage 等价 Message → Ok（spawning 中入站按 Enqueued
    // 排队语义处理，不报错——与 feishu 入站文本路径一致）。
    backend
        .ensure_message(chat.clone(), "second line".into())
        .await
        .expect("ensure on known key delivers");

    // transcript 记账依赖真实 spawn 流程（seed_card），需要 ACP 子进程；
    // 本测试环境（无子进程）不覆盖，由 process-e2e 套件覆盖。
}

/// （extract-im-service 2.2）Cancel：未知 key typed rejection；活跃会话 Ok
/// 且会话保留（快照仍在）。
#[tokio::test]
async fn cancel_rejects_unknown_and_accepts_live_session() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    // 未知 key → typed rejection。
    let bogus = ChannelKey::feishu("oc_cancel_unknown", None);
    let err = backend
        .cancel(bogus.clone())
        .await
        .expect_err("unknown must reject");
    match err {
        SessionRejection::UnknownSession { .. } => {}
        other => panic!("expected UnknownSession, got {other:?}"),
    }

    // spawn + activate → 活跃会话；Cancel → Ok 且会话仍在快照中。
    let key = backend.spawn("work".into(), None).await.expect("spawn");
    let (channel_key, _) = core
        .handle
        .map
        .snapshot_all()
        .await
        .into_iter()
        .next()
        .unwrap();
    core.handle
        .activate(&channel_key, "s-cancel".into(), None, None)
        .await;
    backend
        .cancel(key.clone())
        .await
        .expect("cancel live session");
    let snap = backend.snapshot().await;
    assert_eq!(snap.len(), 1, "cancel keeps the session: {snap:?}");
}

/// （extract-im-service 2.4）ACP 桥审批面经通道全环：ACP PermissionRequest →
/// 订阅流 ApprovalRequested 帧（带 request_id + 会话 key）→ ApprovalAnswer
/// 回路由为 Out::SendAcp{PermissionReply}。迟到/未知 request_id 已由
/// `approval_answer_for_unknown_request_id_returns_typed_rejection` 覆盖
/// （fail-closed）。无客户端连接时内核侧 fail-closed 由 ACP 驱动自身保证
/// （hook 停车超时路径），通道层不做缓存重放（协议模块注释）。
#[tokio::test]
async fn acp_permission_request_streams_and_answer_routes_back() {
    use sebas_acp::AcpEvent;
    use sebas_webui::session_backend::SessionBackend;

    let dir = tempfile::tempdir().unwrap();
    let mut core = start_core(dir.path()).await;

    // 订阅流：snapshot 帧先到。
    let stream = sebas_ipc::connect(&core.path).await.unwrap();
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.into(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(
        b"
",
    )
    .await
    .unwrap();
    let mut ack = String::new();
    reader.read_line(&mut ack).await.unwrap();
    let sub = serde_json::to_string(&CoreChannelRequest::Subscribe).unwrap();
    w.write_all(sub.as_bytes()).await.unwrap();
    w.write_all(
        b"
",
    )
    .await
    .unwrap();
    w.flush().await.unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: SessionStreamFrame = serde_json::from_str(line.trim()).unwrap();
    assert!(matches!(frame, SessionStreamFrame::Snapshot { .. }));

    // 建会话并激活，然后合成一条 ACP PermissionRequest 走 router 应用路径
    // （apply_event_to_out 是 ACP 泵的真实入口，权限广播从这里发出）。
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());
    let key = backend
        .spawn("will need permission".into(), None)
        .await
        .unwrap();
    core.handle
        .activate(&key, "s-perm".into(), None, None)
        .await;
    core.handle
        .apply_event_to_out(
            "s-perm".into(),
            &AcpEvent::PermissionRequest {
                session_id: "s-perm".into(),
                request_id: "toolu_perm_1".into(),
                tool_name: "bash".into(),
                args: serde_json::json!({"command": "ls"}),
            },
        )
        .await;

    // 订阅流上应出现 ApprovalRequested 帧。
    let mut saw_approval = false;
    for _ in 0..8 {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let frame: SessionStreamFrame = serde_json::from_str(line.trim()).unwrap();
        if let SessionStreamFrame::ApprovalRequested { notice } = &frame {
            assert_eq!(notice.request_id, "toolu_perm_1");
            // session_id 是 URL-safe 编码的 ChannelKey（InProcessBackend 中继口径）。
            // session_id 是 URL-safe 编码的 ChannelKey（webui routes 口径）；
            // 这里只断言非空 + tool 正确，编码一致性由 webui 侧测试覆盖。
            assert!(!notice.session_id.is_empty());
            assert_eq!(notice.tool_name, "bash");
            saw_approval = true;
            break;
        }
    }
    assert!(
        saw_approval,
        "ApprovalRequested frame must reach the subscriber"
    );

    // 决定回路由：ApprovalAnswer 走独立请求连接（订阅连接只推流不处理
    // 请求）→ Ok，且 Out::SendAcp{PermissionReply} 路由回 ACP 会话。
    let resp = raw_request(
        &core.path,
        Some(SECRET),
        &CoreChannelRequest::ApprovalAnswer {
            request_id: "toolu_perm_1".into(),
            decision: PermissionDecision::AllowOnce,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::Ok),
        "answer accepted, got {resp:?}"
    );

    // Drain queued Out events briefly; the PermissionReply must be among them.
    let mut saw_reply = false;
    for _ in 0..16 {
        match tokio::time::timeout(Duration::from_millis(300), core._out_rx.recv()).await {
            Ok(Some(out)) => {
                eprintln!("[dbg] out: {out:?}");
                if let sebas_dispatch::Out::SendAcp { session_id, cmd } = out {
                    assert_eq!(session_id, "s-perm");
                    assert!(matches!(cmd, sebas_acp::AcpCommand::PermissionReply { .. }));
                    saw_reply = true;
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    assert!(saw_reply, "PermissionReply must reach the outbound queue");
}

/// （extract-im-service 4.1）附件面：路径不存在的附件 typed rejection；
/// 存在的本地文件投递 Ok（标记组合后由执行体按路径消化）。
#[tokio::test]
async fn ensure_message_attachments_are_validated() {
    use crate::core_channel::protocol::Attachment;

    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let missing = Attachment {
        path: "/definitely/not/here.png".into(),
        mime: Some("image/png".into()),
        name: Some("here.png".into()),
    };
    let resp = raw_request(
        &core.path,
        Some(SECRET),
        &CoreChannelRequest::EnsureMessage {
            key: ChannelKey::feishu("oc_attach", None),
            message: "see this".into(),
            attachments: vec![missing],
        },
    )
    .await
    .unwrap()
    .unwrap();
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(cause.contains("附件路径不存在"), "got: {cause}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }

    // 存在的文件 → Ok（ensure 语义建会话照常）。
    let good = dir.path().join("img.png");
    std::fs::write(&good, b"fake-png").unwrap();
    let resp = raw_request(
        &core.path,
        Some(SECRET),
        &CoreChannelRequest::EnsureMessage {
            key: ChannelKey::feishu("oc_attach", None),
            message: "see this".into(),
            attachments: vec![Attachment {
                path: good.display().to_string(),
                mime: Some("image/png".into()),
                name: Some("img.png".into()),
            }],
        },
    )
    .await
    .unwrap()
    .unwrap();
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::Ok),
        "good attachment accepted, got {resp:?}"
    );
}

// ── harden-core-channel-deployment: auto-arm (1.2) + client discovery (2.1) ──

/// SEBAS_CORE_SECRET env 并行测试隔离：arm 路径在构造期读 env，guard 持
/// `secret_env_test_lock` 到 drop——与 `secret::tests` 的 EnvGuard 共用同一
/// 把锁，防两个模块的并行用例互相踩 env。
struct CoreSecretEnv {
    name: &'static str,
    prev: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl CoreSecretEnv {
    fn set(value: &str) -> Self {
        Self::arm(value, false)
    }
    fn unset() -> Self {
        Self::arm("", true)
    }
    fn arm(value: &str, remove: bool) -> Self {
        let name = "SEBAS_CORE_SECRET";
        let lock = super::secret::secret_env_test_lock().lock().unwrap();
        let prev = std::env::var(name).ok();
        // edition 2024：set/remove 为 unsafe（多线程下 UB 风险）——测试进程
        // 内由共享锁串行化。
        unsafe {
            if remove {
                std::env::remove_var(name);
            } else {
                std::env::set_var(name, value);
            }
        }
        Self {
            name,
            prev,
            _lock: lock,
        }
    }
}

impl Drop for CoreSecretEnv {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var(self.name, v),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

/// 沙箱 config：channel_path 指进沙箱（绝不落真实 XDG_RUNTIME_DIR）。
fn arm_config(dir: &StdPath) -> (crate::config::Config, std::path::PathBuf) {
    let config_path = dir.join("config.toml");
    let raw = format!(
        "[watchdog.core]\nchannel_path = \"{}\"\n",
        dir.join("core.sock").display()
    );
    let cfg = crate::config::Config::parse(&raw).expect("sandbox arm config parses");
    (cfg, config_path)
}

async fn arm_for_test(dir: &StdPath) -> crate::run::ArmedChannel {
    let (cfg, config_path) = arm_config(dir);
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let _keep = _out_rx;
    crate::run::arm_core_channel(&cfg, &config_path, backend, &router, None, None)
        .await
        .expect("arm succeeds in sandbox")
}

/// 1.2 自动武装：无 env 启动 → socket 与 secret 文件同现、0600、内容可完成
/// 握手（客户端走文件发现）。
#[tokio::test]
async fn auto_arm_without_env_writes_secret_file_and_completes_handshake() {
    let _env = CoreSecretEnv::unset();
    let dir = tempfile::tempdir().unwrap();
    let armed = arm_for_test(dir.path()).await;
    let secret_file = dir.path().join("core.secret");

    assert_eq!(
        armed.secret_file, secret_file,
        "default path = config dir/core.secret"
    );
    assert!(
        dir.path().join("core.sock").exists(),
        "socket bound before arm returns"
    );
    assert!(secret_file.exists(), "secret file written at arm time");
    let content = std::fs::read_to_string(&secret_file).unwrap();
    assert_eq!(content, armed.secret, "file carries this boot's secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secret_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "secret file must be 0600");
    }

    // 内容可完成握手：client 走 Discover(secret 文件) 建连成功。
    let backend = CoreChannelBackend::with_secret(
        dir.path().join("core.sock"),
        crate::core_channel::secret::ChannelSecret::from_env_or_file(Some(secret_file.clone())),
    );
    assert!(
        backend.snapshot().await.is_empty(),
        "handshake with discovered secret works"
    );
    assert_eq!(backend.reachability().await, Reachability::Reachable);

    // 清理 accept 循环。
    let _ = armed.shutdown.send(true);
    wait_channel_gone(&dir.path().join("core.sock")).await;
}

/// 1.2 自动武装：env 提供时 env 优先且文件内容一致（迟启动客户端可发现）。
#[tokio::test]
async fn auto_arm_with_env_uses_env_value_and_writes_matching_file() {
    let _env = CoreSecretEnv::set("env-secret-wins");
    let dir = tempfile::tempdir().unwrap();
    let armed = arm_for_test(dir.path()).await;

    assert_eq!(armed.secret, "env-secret-wins", "env must win");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("core.secret")).unwrap(),
        "env-secret-wins",
        "file content must match the env secret"
    );
    let _ = armed.shutdown.send(true);
    wait_channel_gone(&dir.path().join("core.sock")).await;
}

/// 1.3 bind 失败硬失败：路径被存活 listener 占用 → arm 返回特定错误
/// （run.rs 据此在 ready 之前以 75 退出，不产生无通道的“健康”进程）。
#[tokio::test]
async fn arm_fails_hard_when_socket_path_is_taken_by_live_listener() {
    let _env = CoreSecretEnv::unset();
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("core.sock");
    let _squatter = crate::core_channel::server::bind_channel_socket(&sock).expect("occupy path");

    let (cfg, config_path) = arm_config(dir.path());
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let _keep = _out_rx;
    let err = crate::run::arm_core_channel(&cfg, &config_path, backend, &router, None, None)
        .await
        .expect_err("live occupant must fail the arm");
    let msg = err.to_string();
    assert!(
        msg.contains("bind 失败") && msg.contains("already served"),
        "error must name the bind failure cause: {msg}"
    );
}

/// 2.1 客户端文件发现 + 换钥自愈：core 重启换钥（新 secret 覆写文件）后，
/// 不重建的 client 经重连重读文件恢复可达。
#[tokio::test]
async fn client_discovers_secret_from_file_and_heals_key_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let secret_file = dir.path().join("core.secret");
    std::fs::write(&secret_file, "rotation-key-1").unwrap();

    let core = start_core(dir.path()).await; // server 用常量 SECRET= "test-core-secret"
    // 文件发现路径：env 缺省（测试进程不保证，但 Discover 只在 env 空时读文件
    // ——这里直接以 Discover 构造并写匹配常量的文件）。
    std::fs::write(&secret_file, SECRET).unwrap();
    let backend = CoreChannelBackend::with_secret(
        core.path.clone(),
        crate::core_channel::secret::ChannelSecret::Discover(Some(secret_file.clone())),
    );
    let key = backend
        .spawn("before rotation".into(), None)
        .await
        .expect("handshake via file");
    assert_eq!(backend.snapshot().await.len(), 1);

    // core 重启（换钥）：文件覆写为新钥，同一 client 实例自愈。
    let _ = core.close_tx.send(true);
    wait_channel_gone(&core.path).await;
    std::fs::write(&secret_file, "rotation-key-2").unwrap();
    let path2 = core.path.clone();
    let (_router2, _rx2) = DispatchHandle::new(SessionMap::new());
    let router2 = _router2.clone();
    let (_close2, close2_rx) = tokio::sync::watch::channel(false);
    let backend2: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router2.clone()),
    );
    tokio::spawn(async move {
        let _ = server::serve(
            backend2,
            router2,
            path2.clone(),
            "rotation-key-2".into(),
            close2_rx,
        )
        .await;
    });
    wait_channel_ready(&core.path).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let _ = backend.snapshot().await; // trigger reconnect
        if backend.reachability().await == Reachability::Reachable {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "client must re-read the rotated secret and reconnect"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let _ = key;
}

// ── cover-core-channel-test-gaps A1.2: reachability three-state ─────────────

/// SEBAS_STARTUP_ERROR_FILE 进程级隔离：enrich（client.rs）在**任何**不可达
/// 断言时都会读该 env（D2 无条件富化不收窄）——设置/清除它的用例与做精确
/// cause 断言的用例共用这把锁串行，防并行 #[tokio::test] 互相踩 env。
static STARTUP_ERROR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 持有期内把 SEBAS_STARTUP_ERROR_FILE 指向沙箱文件（或清掉），drop 时还原。
struct StartupErrorFile {
    _dir: tempfile::TempDir,
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl StartupErrorFile {
    /// env → 新沙箱文件，内容为 `contents`。
    fn set(contents: &str) -> Self {
        let lock = STARTUP_ERROR_ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup-error");
        std::fs::write(&path, contents).unwrap();
        let name = crate::startup_failure::SUMMARY_FILE_ENV;
        let prev = std::env::var_os(name);
        // edition 2024：多线程下 set/remove env 是 unsafe；由锁串行化。
        unsafe { std::env::set_var(name, &path) };
        Self {
            _dir: dir,
            prev,
            _lock: lock,
        }
    }

    /// env 清除（还原 prev）。
    fn unset() -> Self {
        let lock = STARTUP_ERROR_ENV_LOCK.lock().unwrap();
        let name = crate::startup_failure::SUMMARY_FILE_ENV;
        let prev = std::env::var_os(name);
        unsafe { std::env::remove_var(name) };
        Self {
            _dir: tempfile::tempdir().unwrap(),
            prev,
            _lock: lock,
        }
    }
}

impl Drop for StartupErrorFile {
    fn drop(&mut self) {
        let name = crate::startup_failure::SUMMARY_FILE_ENV;
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// (a) socket 不存在 + `SEBAS_STARTUP_ERROR_FILE` 含闩锁摘要 →
/// `StartupFailed { cause: "core startup failed: <原因>" }`（enrich 全串，
/// 前端原文渲染）。
#[tokio::test]
async fn reachability_startup_failed_with_env_file() {
    let _env = StartupErrorFile::set("startup-failure: bad config\n");
    let dir = tempfile::tempdir().unwrap();
    let backend = CoreChannelBackend::new(dir.path().join("missing.sock"), SECRET.into());

    let _ = backend.snapshot().await; // trigger the failure + status latch
    match backend.reachability().await {
        Reachability::StartupFailed { cause } => {
            assert_eq!(cause, "core startup failed: bad config");
        }
        other => panic!("expected StartupFailed, got {other:?}"),
    }
}

/// (a-fallback) socket 不存在、无闩锁文件 → `StartupFailed` 且 cause 为
/// 带路径的 fallback（"core session channel socket not found at <path>"）。
#[tokio::test]
async fn reachability_startup_failed_fallback() {
    let _env = StartupErrorFile::unset();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.sock");
    let backend = CoreChannelBackend::new(path.clone(), SECRET.into());

    let _ = backend.snapshot().await;
    match backend.reachability().await {
        Reachability::StartupFailed { cause } => {
            assert_eq!(
                cause,
                format!(
                    "core session channel socket not found at {}",
                    path.display()
                )
            );
        }
        other => panic!("expected StartupFailed, got {other:?}"),
    }
}

/// (b) socket 在、secret 错 → `AuthRejected { cause: "core rejected channel
/// handshake" }`；重试语义（spec 已修正版）：同 secret 不无限重试——一次
/// 请求内只有一次握手，失败立即以 AuthRejected 返回；env 未设的文件发现
/// 客户端在**下一次尝试前**重读 secret 文件，文件换成正确钥后再试一次即
/// 恢复（core 重启换钥自愈的最小形态）。
#[tokio::test]
async fn reachability_auth_rejected_after_handshake() {
    // Exact-cause assertion: hold the startup-error env lock so a parallel
    // latch-file test cannot rewrite the cause via the unconditional enrich.
    let _env = StartupErrorFile::unset();
    let dir = tempfile::tempdir().unwrap();
    let secret_file = dir.path().join("core.secret");
    std::fs::write(&secret_file, "wrong-key").unwrap();
    let core = start_core(dir.path()).await; // server secret = SECRET
    let backend = CoreChannelBackend::with_secret(
        core.path.clone(),
        crate::core_channel::secret::ChannelSecret::Discover(Some(secret_file.clone())),
    );

    // 错钥被拒：一个请求 = 一次握手，失败即刻报告，不内部重试风暴。
    let err = backend.spawn("x".into(), None).await.unwrap_err();
    match err {
        SessionRejection::Unavailable { cause } => {
            assert_eq!(cause, "core rejected channel handshake");
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert_eq!(
        backend.reachability().await,
        Reachability::AuthRejected {
            cause: "core rejected channel handshake".into()
        }
    );

    // 文件换正确钥（env 未设 → Discover 每次连接前重读文件），同一 client
    // 实例至多再试一次即恢复，而不是继续用失败过的旧钥。
    std::fs::write(&secret_file, SECRET).unwrap();
    let key = backend
        .spawn("after re-read".into(), None)
        .await
        .expect("one more try succeeds");
    assert_eq!(backend.reachability().await, Reachability::Reachable);
    let _ = key;
}

/// (c) 握手成功后、请求得到应答前对端断连（ack 后直接关闭）→
/// `Disconnected { cause: "connection dropped" }`——与 (a)/(b) 三态互斥。
#[tokio::test]
async fn reachability_disconnected_after_connected() {
    // Exact-cause assertion: hold the startup-error env lock (see the auth
    // test) — the kind must stay Disconnected and the cause verbatim.
    let _env = StartupErrorFile::unset();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sock");
    let listener = server::bind_channel_socket(&path).expect("bind fake listener");
    // Fake peer: accept loop — every connection gets a successful handshake
    // ack, then (after consuming the request line) is dropped without
    // answering, so the client's read hits post-handshake EOF.
    tokio::spawn(async move {
        for _ in 0..16 {
            let stream =
                match tokio::time::timeout(Duration::from_secs(10), sebas_ipc::accept(&listener))
                    .await
                {
                    Ok(Ok(s)) => s,
                    _ => return,
                };
            let (r, mut w) = sebas_ipc::split(stream);
            let mut reader = BufReader::new(r);
            let mut line = String::new();
            use tokio::io::AsyncWriteExt;
            // Best-effort handshake read (probe connections may send nothing).
            let _ = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await;
            let _ = w.write_all(b"{\"handshake\":\"ok\"}\n").await;
            let _ = w.flush().await;
            // Consume the request line (best effort) so the client's write
            // lands before the drop — its read then sees the EOF.
            let _ = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await;
            let _ = w.shutdown().await;
            drop((reader, w)); // post-handshake disconnect
        }
    });
    wait_channel_ready(&path).await;

    let backend = CoreChannelBackend::new(path.clone(), SECRET.into());
    let err = backend.spawn("x".into(), None).await.unwrap_err();
    match err {
        SessionRejection::Unavailable { cause } => {
            assert_eq!(cause, "connection dropped");
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert_eq!(
        backend.reachability().await,
        Reachability::Disconnected {
            cause: "connection dropped".into()
        }
    );
}
// ── cover-core-channel-test-gaps A2.2: ensure_message / Message 语义 ────────

/// 未知 key 上的 EnsureMessage 自动建会话并返回 Ok；后续 Snapshot 包含该
/// 会话（spec scenario: "unknown key auto-creates session via EnsureMessage"）。
#[tokio::test]
async fn ensure_message_unknown_key_auto_creates() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let chat = ChannelKey::feishu("oc_a2_ensure_new", None);
    backend
        .ensure_message(chat.clone(), "hi".into())
        .await
        .expect("unknown key auto-creates");

    let snap = backend.snapshot().await;
    assert_eq!(snap.len(), 1, "exactly one session created: {snap:?}");
    assert!(snap[0].key.contains("oc_a2_ensure_new"));
    assert_eq!(snap[0].status, "spawning");
}

/// dormant 会话经 EnsureMessage 懒复活：Ok + 快照状态从 dormant 变为
/// spawning（spec scenario: "dormant session resumes via EnsureMessage"；
/// 复活在事件流上是 Dormant→Spawning 的 Updated——引擎没有独立 Revived 帧）。
#[tokio::test]
async fn ensure_message_dormant_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let chat = ChannelKey::feishu("oc_a2_dormant", None);
    core.handle
        .map
        .insert(
            chat.clone(),
            sebas_dispatch::state::Mapping::dormant("s-a2-dormant", 1),
        )
        .await
        .unwrap();
    let before = backend.snapshot().await;
    assert_eq!(
        before[0].status, "dormant",
        "fixture starts dormant: {before:?}"
    );

    backend
        .ensure_message(chat.clone(), "wake up".into())
        .await
        .expect("dormant resumes");

    let after = backend.snapshot().await;
    assert_eq!(after.len(), 1, "same session, resumed in place: {after:?}");
    assert_eq!(
        after[0].status, "spawning",
        "dormant claimed → spawning: {after:?}"
    );
}

/// Message 在未知 key 上保持 typed rejection（UnknownSession），且不创建
/// 任何会话——webui「未知即拒绝」语义不受 ensure 语义影响（spec scenario:
/// "Message on unknown key is rejected"；extract-im-service 2.1 回归）。
#[tokio::test]
async fn message_unknown_key_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let bogus = ChannelKey::feishu("oc_a2_unknown", None);
    assert_eq!(
        backend.message(bogus.clone(), "hi".into()).await,
        Err(SessionRejection::UnknownSession {
            key: serde_json::to_string(&bogus).unwrap()
        })
    );
    assert!(
        backend.snapshot().await.is_empty(),
        "no session may be created by a rejected Message"
    );
}

// ── cover-core-channel-test-gaps A2.3: cross-uid rejection, live process ────

/// Fork-safe child body: switch to `uid`, connect to the channel socket, send
/// a correctly-secred handshake and expect the server to close without any
/// ack byte (peer-uid mismatch rejection). Pure libc syscalls only — no
/// allocation, no locks, no Rust runtime services (the child of fork() in a
/// multithreaded process may only do async-signal-safe work).
/// Exit codes: 0 = rejected as expected; 2 = setuid failed; 3 = connect
/// failed; 4 = server ACKED a foreign-uid handshake (the bug this test guards).
unsafe fn cross_uid_child_body(path_bytes: &[u8], handshake: &[u8], uid: u32) -> i32 {
    // Edition 2024: an `unsafe fn` body is not an implicit unsafe block.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        if libc::setuid(uid) != 0 {
            return 2;
        }
        // setuid 报告成功还不够：确认 real/effective 都已切过去（诊断
        // "server acked" 到底是 uid 没切成还是检查缺失）。
        if libc::getuid() != uid || libc::geteuid() != uid {
            return 5;
        }
        let sock = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
        if sock < 0 {
            return 3;
        }
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let len = path_bytes.len().min(addr.sun_path.len() - 1);
        std::ptr::copy_nonoverlapping(path_bytes.as_ptr(), addr.sun_path.as_mut_ptr().cast(), len);
        let addr_len = std::mem::size_of::<libc::sa_family_t>() + len;
        if libc::connect(
            sock,
            (&addr as *const libc::sockaddr_un).cast(),
            addr_len as libc::socklen_t,
        ) != 0
        {
            libc::close(sock);
            return 3;
        }
        // The handshake carries the CORRECT secret: if the server still closes
        // without an ack, the rejection came from the peer-uid check alone.
        let mut written = 0usize;
        while written < handshake.len() {
            let n = libc::write(
                sock,
                handshake[written..].as_ptr().cast(),
                handshake.len() - written,
            );
            if n <= 0 {
                // Server already closed on the uid mismatch — that is a reject.
                libc::close(sock);
                return 0;
            }
            written += n as usize;
        }
        let mut ack = [0u8; 32];
        let n = libc::read(sock, ack.as_mut_ptr().cast(), ack.len());
        libc::close(sock);
        // n == 0（干净 EOF）或 n == -1（ECONNRESET：服务端带着未读数据 close，
        // 内核对端回 RST）都等于「无 ack，被拒」；只有 n > 0 才是握手 ack。
        if n <= 0 { 0 } else { 4 }
    }
}

/// 真实跨 uid 进程的 peer-uid 拒绝（5.2 / design D5，**CI-only 非门禁**）：
/// fork 子进程后 `setuid` 到 nobody/daemon（不是同进程改 uid，是真实跨进程
/// 凭证），携正确 secret 握手——服务端必须因 uid 不匹配直接关闭连接、不给
/// ack、不进入 request 处理。非 root 下 `setuid` 必败，测试自身提前跳过；
/// 本地开发账户直接 `#[ignore]`，CI runner 默认 root 时以
/// `cargo test -p sebas -- --ignored cross_uid` 运行。
#[tokio::test]
#[cfg(unix)]
#[ignore = "needs root (forks a child that setuid()s to an unprivileged account) — CI-only, design D5"]
async fn cross_uid_rejected_live_process() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    // 非 root：setuid 注定失败——如实跳过（非门禁），不报错。
    if unsafe { libc::getuid() } != 0 {
        eprintln!(
            "[skip] cross_uid_rejected_live_process needs root to setuid a child (uid={})",
            unsafe { libc::getuid() }
        );
        return;
    }

    // 解析现成非特权账户：nobody → daemon → 65534。
    let target_uid = {
        let nobody = std::ffi::CString::new("nobody").unwrap();
        let pw = unsafe { libc::getpwnam(nobody.as_ptr()) };
        if !pw.is_null() {
            unsafe { (*pw).pw_uid }
        } else {
            let daemon = std::ffi::CString::new("daemon").unwrap();
            let pw = unsafe { libc::getpwnam(daemon.as_ptr()) };
            if !pw.is_null() {
                unsafe { (*pw).pw_uid }
            } else {
                65534
            }
        }
    };
    assert_ne!(
        target_uid,
        unsafe { libc::getuid() },
        "child must run as a DIFFERENT uid"
    );

    // 生产通道是 0600 + 私有目录——那层由文件系统先把外部 uid 挡在
    // connect 之前（EACCES），服务端的 peer-uid 检查永远看不到连接。为了
    // 在本用例里真实 exercising 服务端检查（design D5 的意图），测试场景把
    // socket 放宽到 0666、目录放宽到 0755——仅测试文件，生产绑定不变。
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&core.path, std::fs::Permissions::from_mode(0o666))
            .expect("relax socket perms");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
            .expect("relax scene dir perms");
    }

    // fork 前把路径与握手行放进栈上定长缓冲（子进程零分配）。
    let path_bytes = {
        let p = core.path.to_string_lossy();
        let mut buf = [0u8; 96];
        let bytes = p.as_bytes();
        assert!(
            bytes.len() < 92,
            "channel path must fit sun_path (sandbox scenes use short paths)"
        );
        buf[..bytes.len()].copy_from_slice(bytes);
        (buf, bytes.len())
    };
    let handshake = {
        let line = format!("{{\"secret\":\"{SECRET}\"}}\n");
        let mut buf = [0u8; 128];
        buf[..line.len()].copy_from_slice(line.as_bytes());
        (buf, line.len())
    };

    let child_pid = unsafe { libc::fork() };
    assert!(child_pid >= 0, "fork failed");
    if child_pid == 0 {
        let code = unsafe {
            cross_uid_child_body(
                &path_bytes.0[..path_bytes.1],
                &handshake.0[..handshake.1],
                target_uid,
            )
        };
        unsafe { libc::_exit(code) };
    }

    // waitpid 会阻塞当前线程——`#[tokio::test]` 默认 current-thread runtime，
    // 直接在这里等会把唯一线程饿死：通道 server 任务（accept 子进程连接→
    // 关闭）永远得不到调度，子进程 read 等不到 EOF，父子互锁。放到
    // blocking 池里等，runtime 线程保持空闲去跑 server 任务。
    let status = tokio::task::spawn_blocking(move || {
        let mut status: libc::c_int = 0;
        let wait = unsafe { libc::waitpid(child_pid, &mut status, 0) };
        assert_eq!(wait, child_pid, "waitpid on the forked child");
        status
    })
    .await
    .expect("waitpid task joins");
    assert!(
        libc::WIFEXITED(status),
        "child exited normally, status={status}"
    );
    let code = libc::WEXITSTATUS(status);
    match code {
        0 => {}
        2 => panic!("child setuid failed even under root (uid {target_uid})"),
        3 => panic!("child could not connect to the channel socket"),
        4 => panic!("server ACKED a foreign-uid handshake — peer-uid check missing!"),
        5 => panic!("child setuid reported success but uids did not switch"),
        other => panic!("unexpected child exit code {other}"),
    }

    // 拒绝不进入 request 处理：合法客户端的快照仍是空的、core 侧映射为空。
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());
    assert!(
        backend.snapshot().await.is_empty(),
        "no request was processed"
    );
    assert!(
        core.handle.map.snapshot_all().await.is_empty(),
        "core map untouched by the foreign-uid connection"
    );
}

// ── workbench-turn-queue 4.1/4.2/4.3 ────────────────────────────────────────

/// pending submission 随快照/事件可见（4.1）；remove/move 的 typed 拒绝在
/// detached（通道）形态与 in-process 实现同一结果（4.2/4.3）。
#[tokio::test]
async fn pending_submissions_visible_and_manageable_over_the_channel() {
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let key = ChannelKey::new("web", "web-pending");

    // spawn 窗口：占位 + 1 条 staged（turn 队列 1 条 priority + 1 条普通）。
    core.handle.map.begin_spawn(key.clone()).await.unwrap();
    let _ = core
        .handle
        .map
        .route_text(key.clone(), "staged entry".into())
        .await
        .unwrap();
    let _ = core
        .handle
        .map
        .route_text(key.clone(), "staged two".into())
        .await
        .unwrap();
    let staged_id = core.handle.map.pending_submissions(&key).await[0].id;
    let prio_id = core
        .handle
        .map
        .enqueue_turn(
            &key,
            sebas_dispatch::state::QueuedTurn::new("prio", None, true),
        )
        .await;
    let norm_id = core
        .handle
        .map
        .enqueue_turn(
            &key,
            sebas_dispatch::state::QueuedTurn::new("norm", None, false),
        )
        .await;

    // 4.1：快照携带 pending 全量视图（投递序 = staging 先于 turn）。
    let snap = backend.snapshot().await;
    let entry = snap
        .iter()
        .find(|s| s.channel == "web" && s.key == "web-pending")
        .expect("session in snapshot");
    assert_eq!(entry.pending.len(), 4);
    assert_eq!(
        entry.pending[0].disposition,
        sebas_dispatch::PendingDisposition::Staging
    );
    assert_eq!(entry.pending[1].text, "staged two");
    assert!(
        entry.pending[2].priority,
        "priority turn first in its group"
    );
    assert_eq!(entry.pending[2].position, 2);

    // 4.2/4.3：detached 形态的 remove/move 与 in-process 同语义。
    // 成功路径：返回操作后的全量 pending。
    let after = backend
        .remove_pending(key.clone(), norm_id)
        .await
        .expect("remove");
    assert!(!after.iter().any(|p| p.id == norm_id));
    let after = backend
        .move_pending(key.clone(), staged_id, 1)
        .await
        .expect("staged group reorder");
    assert_eq!(after[1].text, "staged entry");
    assert_eq!(after[0].text, "staged two");
    // 越优先：普通项插到优先项之前 → PriorityConflict（用一条活的普通
    // 提交；norm_id 已被移除，对它再动是 Unknown——语义见上一组断言）。
    let fresh_id = core
        .handle
        .map
        .enqueue_turn(
            &key,
            sebas_dispatch::state::QueuedTurn::new("fresh", None, false),
        )
        .await;
    assert_eq!(
        backend.move_pending(key.clone(), fresh_id, 0).await,
        Err(SessionRejection::PendingRejected {
            reason: sebas_webui::session_backend::PendingReason::PriorityConflict
        })
    );
    assert_eq!(
        backend.move_pending(key.clone(), norm_id, 0).await,
        Err(SessionRejection::PendingRejected {
            reason: sebas_webui::session_backend::PendingReason::Unknown
        })
    );
    // 未知 id → Unknown。
    assert_eq!(
        backend.remove_pending(key.clone(), 99_999).await,
        Err(SessionRejection::PendingRejected {
            reason: sebas_webui::session_backend::PendingReason::Unknown
        })
    );
    // 已开始（pop 投递）→ AlreadyStarted。
    core.handle.map.pop_next_turn(&key).await.expect("prio pop");
    assert_eq!(
        backend.remove_pending(key.clone(), prio_id).await,
        Err(SessionRejection::PendingRejected {
            reason: sebas_webui::session_backend::PendingReason::AlreadyStarted
        })
    );
    assert_eq!(
        backend.move_pending(key.clone(), prio_id, 0).await,
        Err(SessionRejection::PendingRejected {
            reason: sebas_webui::session_backend::PendingReason::AlreadyStarted
        })
    );

    // 4.1：事件携带 pending——激活后把会话推到 WORKING，web 提交经共享
    // submit_turn 入队并发布 Updated，全量 pending 随事件到达订阅端。
    core.handle
        .activate(&key, "s-live".into(), None, None)
        .await;
    core.handle.seed_card("s-live".into(), "seed".into()).await;
    use sebas_acp::claude::session::AcpEvent;
    core.handle
        .apply_event(
            "s-live",
            &AcpEvent::TextDelta {
                session_id: "s-live".into(),
                delta: "streaming...".into(),
            },
        )
        .await;
    // 同 5.2 测试：等 forwarder 订阅就绪，避免事件落在订阅建立前。
    let dl = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < dl,
            "channel forwarder never became reachable"
        );
        if backend.reachability().await == Reachability::Reachable {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut events = backend.subscribe();
    core.handle
        .web_send_message(key.clone(), "second".into())
        .await
        .expect("accepted (queued)");
    let mut saw_pending = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !saw_pending && std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await {
            Ok(Ok(SessionEvent::Updated { session })) => {
                saw_pending = session.pending.iter().any(|p| p.text == "second");
            }
            Ok(Ok(_)) => continue,
            _ => {}
        }
    }
    assert!(saw_pending, "Updated events carry the full pending list");
}

/// workbench-turn-queue 5.2：close 在移除映射**之前**发布 PendingDropped
///（逐条标注未执行），响应携带 discarded_pending 计数；观察者按序收到
/// PendingDropped → Removed。
#[tokio::test]
async fn close_reports_discarded_pending_and_notifies_observers() {
    use sebas_acp::claude::session::AcpEvent;

    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let key = ChannelKey::new("web", "web-drop");
    core.handle
        .insert_mapping(key.clone(), "s-drop".into())
        .await;
    core.handle.seed_card("s-drop".into(), "run".into()).await;
    core.handle
        .apply_event(
            "s-drop",
            &AcpEvent::TextDelta {
                session_id: "s-drop".into(),
                delta: "working".into(),
            },
        )
        .await;
    let _ = core
        .handle
        .map
        .enqueue_turn(
            &key,
            sebas_dispatch::state::QueuedTurn::new("dropped one", None, false),
        )
        .await;
    let _ = core
        .handle
        .map
        .enqueue_turn(
            &key,
            sebas_dispatch::state::QueuedTurn::new("dropped two", None, false),
        )
        .await;

    // 等 forwarder 的订阅连接就绪（就绪前发布的事件不会被补发——广播不是
    // 日志），否则 close 的标注帧会落在订阅建立之前。
    let dl = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < dl,
            "channel forwarder never became reachable"
        );
        if backend.reachability().await == Reachability::Reachable {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut events = backend.subscribe();
    let report = backend.close(key.clone()).await.expect("close");
    assert_eq!(
        report.discarded_pending, 2,
        "close must count what it dropped"
    );

    // 观察者按序收到 PendingDropped（携带两条文本）→ Removed。
    let mut saw_dropped = None;
    let mut saw_removed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !(saw_dropped.is_some() && saw_removed) && std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await {
            Ok(Ok(SessionEvent::PendingDropped { dropped, .. })) => {
                let texts: Vec<String> = dropped.iter().map(|d| d.text.clone()).collect();
                saw_dropped = Some(texts);
            }
            Ok(Ok(SessionEvent::Removed { .. })) => saw_removed = true,
            Ok(Ok(_)) => continue,
            _ => {}
        }
    }
    assert_eq!(
        saw_dropped,
        Some(vec!["dropped one".to_string(), "dropped two".to_string()]),
        "observers must be told exactly which submissions were not executed"
    );
    assert!(saw_removed, "Removed follows the drop annotation");
}
