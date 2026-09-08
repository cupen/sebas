//! State 三件 contract tests（cover-core-channel-test-gaps A2.1）。
//!
//! 功能：状态库通道面 / 子功能：StateSnapshot / StateMutation / StateSubscribe
//! 的 channel 转发语义。
//!
//! 独立进程（tests/ 集成测试）里注入 fake state-store engine（design D7：
//! 接口与 `StateStoreEngine` 完全匹配），验证 channel 把请求路由到 engine、
//! engine 响应能回包、mutation 失败走 typed rejection、订阅在快照帧之后推
//! Changed 帧。与 `state_subscription_test.rs`（真 DB 引擎）同理：lib 单测
//! 进程不能初始化全局 engine——会污染 provider/spawn_env 等依赖「engine 未
//! 初始化走文件回退」的并行测试（state_store.rs 的 ENGINE 是每进程一次的
//! OnceLock），所以放到这里。

use sebas::core_channel::client::CoreChannelBackend;
use sebas::core_channel::protocol::{ChannelHandshake, CoreChannelRequest, CoreChannelResponse};
use sebas::core_channel::server;
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::DispatchHandle;
use sebas_webui::session_backend::SessionRejection;
use sebas_webui::SessionBackend;
use std::path::Path as StdPath;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SECRET: &str = "state-contract-secret";

/// fake engine 的可检视内部状态（各用例用独立 path 键位，避免并行互踩）。
#[derive(Default)]
struct FakeStateInner {
    settings: std::sync::Mutex<Option<serde_json::Value>>,
    projects: std::sync::Mutex<Vec<serde_json::Value>>,
}

struct TestCore {
    path: std::path::PathBuf,
    close_tx: tokio::sync::watch::Sender<bool>,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        let _ = self.close_tx.send(true);
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn start_core(dir: &StdPath) -> TestCore {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let path = dir.join("core.sock");
    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    let serve_path = path.clone();
    let serve_router = router.clone();
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(serve_router.clone()),
    );
    tokio::spawn(async move {
        let _ = server::serve(backend, serve_router, serve_path, SECRET.into(), close_rx).await;
    });
    for _ in 0..250 {
        if path.exists() {
            return TestCore { path, close_tx };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("core session channel socket never appeared");
}

/// Raw one-shot request helper (same shape as the lib tests').
async fn raw_request(
    path: &StdPath,
    req: &CoreChannelRequest,
) -> std::io::Result<Option<String>> {
    let stream = sebas_ipc::connect(path).await?;
    let (r, mut w) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.to_string(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    let mut ack = String::new();
    reader.read_line(&mut ack).await?;
    assert!(ack.contains("handshake"), "handshake must succeed: {ack}");

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

/// 进程级只初始化一次 fake engine（Once::call_once 防并行用例的
/// check-then-act 竞态）；返回共享 inner 供用例预置/检查数据。
fn init_fake_state_store() -> Arc<FakeStateInner> {
    use std::sync::{Once, OnceLock};
    static INIT: Once = Once::new();
    static INNER: OnceLock<Arc<FakeStateInner>> = OnceLock::new();
    let inner = INNER.get_or_init(|| Arc::new(FakeStateInner::default()));
    INIT.call_once(|| {
        sebas_dispatch::state_store::init_engine(Box::new(FakeStateEngineImpl {
            inner: inner.clone(),
        }));
    });
    inner.clone()
}

/// The real `async_trait` impl (a named struct so the OnceLock init above can
/// box it without leaking async-trait generics through the helper).
struct FakeStateEngineImpl {
    inner: Arc<FakeStateInner>,
}

#[async_trait::async_trait]
impl sebas_dispatch::state_store::StateStoreEngine for FakeStateEngineImpl {
    async fn load_persisted_state(&self) -> sebas_dispatch::state_store::PersistedState {
        sebas_dispatch::state_store::PersistedState::default()
    }
    async fn save_persisted_state(
        &self,
        _state: sebas_dispatch::state_store::PersistedState,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String> {
        Ok(self.inner.settings.lock().unwrap().clone())
    }
    async fn save_settings(&self, cfg: serde_json::Value) -> Result<(), String> {
        *self.inner.settings.lock().unwrap() = Some(cfg);
        Ok(())
    }
    async fn load_projects(&self) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.inner.projects.lock().unwrap().clone())
    }
    async fn save_projects(&self, projects: Vec<serde_json::Value>) -> Result<(), String> {
        *self.inner.projects.lock().unwrap() = projects;
        Ok(())
    }
    async fn add_project(&self, path: &str, name: &str, added_at: i64) -> Result<(), String> {
        let mut g = self.inner.projects.lock().unwrap();
        if g.iter().any(|p| p.get("path").and_then(|v| v.as_str()) == Some(path)) {
            return Err(format!("add: project '{path}' 已存在"));
        }
        g.push(serde_json::json!({"path": path, "name": name, "added_at": added_at}));
        Ok(())
    }
    async fn remove_project(&self, path: &str) -> Result<bool, String> {
        let mut g = self.inner.projects.lock().unwrap();
        let before = g.len();
        g.retain(|p| p.get("path").and_then(|v| v.as_str()) != Some(path));
        Ok(g.len() != before)
    }
}

/// StateSnapshot 返回 engine 持有的当前快照（spec scenario:
/// "StateSnapshot returns current snapshot"）。
#[tokio::test]
async fn state_snapshot_returns_current() {
    let inner = init_fake_state_store();
    inner
        .projects
        .lock()
        .unwrap()
        .push(serde_json::json!({"path": "/tmp/state-snap-proj", "name": "snap", "added_at": 1}));
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    // Trait 层：snapshot payload 与 engine 持有一致。
    let payload = backend
        .state_snapshot("projects")
        .await
        .expect("engine initialized → snapshot delivered");
    assert!(
        payload["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["path"] == "/tmp/state-snap-proj"),
        "payload: {payload}"
    );

    // 协议层：StateSnapshot 请求 → StateSnapshot 帧回包。
    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateSnapshot {
            domain: "projects".into(),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::StateSnapshot { domain, payload } => {
            assert_eq!(domain, "projects");
            assert!(payload.get("projects").is_some(), "payload: {payload}");
        }
        other => panic!("expected StateSnapshot, got {other:?}"),
    }
}

/// StateMutation 应用变更并回 `StateMutationOk`；下一次 StateSnapshot 看到
/// 新值（spec scenario: "StateMutation applies change"）。
#[tokio::test]
async fn state_mutation_applies_change() {
    let _inner = init_fake_state_store();
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "projects".into(),
            payload: serde_json::json!({
                "op": "add",
                "path": "/tmp/state-mut-proj",
                "name": "mut"
            }),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::StateMutationOk),
        "mutation accepted, got {resp:?}"
    );

    let payload = backend
        .state_snapshot("projects")
        .await
        .expect("snapshot after mutation");
    assert!(
        payload["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["path"] == "/tmp/state-mut-proj"),
        "next snapshot sees the mutation: {payload}"
    );
}

/// StateMutation 携带不可应用的 payload → `Rejected` typed rejection（不静默
/// 吞错），engine 状态未变（spec scenario:
/// "StateMutation rejected does not silently swallow"）。真实错误路径：
/// remove 一个不存在的 project。
#[tokio::test]
async fn state_mutation_rejected_does_not_silently_swallow() {
    let inner = init_fake_state_store();
    inner
        .projects
        .lock()
        .unwrap()
        .push(serde_json::json!({"path": "/tmp/state-rej-proj", "name": "rej", "added_at": 1}));
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "projects".into(),
            payload: serde_json::json!({"op": "remove", "path": "/tmp/state-not-there"}),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(
                    cause.contains("不存在"),
                    "rejection must name the failed removal: {cause}"
                );
            }
            other => panic!("expected Unavailable rejection, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }

    // engine 状态未变：原 project 仍在、被 remove 的目标从未出现（共享
    // fake engine 下并行用例会各自 add 各自的 path，所以按键断言不计长度）。
    let payload = backend.state_snapshot("projects").await.unwrap();
    let arr = payload["projects"].as_array().unwrap();
    assert!(
        arr.iter().any(|p| p["path"] == "/tmp/state-rej-proj"),
        "seeded project still there: {payload}"
    );
    assert!(
        arr.iter().all(|p| p["path"] != "/tmp/state-not-there"),
        "rejected remove must not have mutated anything: {payload}"
    );
}

/// StateSubscribe 先收全域快照帧，订阅期间的 mutation 以 Changed 帧到达
/// （spec scenario: "StateSubscribe delivers mutations after snapshot"）。
#[tokio::test]
async fn state_subscribe_delivers_mutations_after_snapshot() {
    let _inner = init_fake_state_store();
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

    // 首帧：全域快照。
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: sebas::core_channel::protocol::StateStreamFrame =
        serde_json::from_str(line.trim()).unwrap();
    assert!(
        matches!(
            &frame,
            sebas::core_channel::protocol::StateStreamFrame::Snapshot { .. }
        ),
        "first frame must be the snapshot, got {frame:?}"
    );

    // 订阅期间的 mutation → Changed 帧。服务端在写出快照帧之后才建立广播
    // 订阅，无订阅者的 notify 是 no-op——带重试地 notify（帧经 100ms 合并
    // 窗口后到达）。
    let mut saw_changed = false;
    for _ in 0..25 {
        sebas_dispatch::state_store::notify_change("projects");
        line.clear();
        match tokio::time::timeout(Duration::from_millis(200), reader.read_line(&mut line)).await {
            Ok(Ok(0)) | Err(_) => continue, // no frame yet (or merge window) → notify again
            Ok(Ok(_)) => {
                let frame: sebas::core_channel::protocol::StateStreamFrame =
                    serde_json::from_str(line.trim()).unwrap();
                if let sebas::core_channel::protocol::StateStreamFrame::Changed { scope } = &frame {
                    assert_eq!(scope, "projects", "changed frame carries the scope");
                    saw_changed = true;
                    break;
                }
            }
            Ok(Err(e)) => panic!("stream read failed: {e}"),
        }
    }
    assert!(
        saw_changed,
        "mutation during the subscription must arrive as a Changed frame"
    );
}
