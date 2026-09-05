//! 状态订阅链路集成测试（add-state-store 4.2/5.3/5.4）。
//!
//! 独立进程（tests/ 集成测试）里初始化全局状态引擎并验证完整闭环：
//!
//! mutation（DB 引擎） → `notify_change` → core channel 服务端广播
//!   → StateSubscribe 订阅端收到 Changed 帧。
//!
//! lib 单测不能初始化全局 engine（会污染 spawn_env 等依赖「engine 未
//! 初始化走文件路径」的并行测试），所以放到这里。
//!
//! 整个文件跑在 unix domain socket 上（tokio UnixStream + `.sock` 路径），
//! 仅在 unix 编译；Windows 上该测试目标为空。
#![cfg(unix)]

use sebas::core_channel::protocol::{
    ChannelHandshake, CoreChannelRequest, StateStreamFrame,
};
use sebas::core_channel::server;
use sebas::sebas_state::engine::DbStateEngine;
use sebas::sebas_state::writer::StateWriter;
use sebas_dispatch::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const SECRET: &str = "test-state-secret";

struct TestCore {
    path: std::path::PathBuf,
    close_tx: tokio::sync::watch::Sender<bool>,
    _handle: DispatchHandle,
    _out_rx: tokio::sync::mpsc::Receiver<sebas_dispatch::Out>,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        let _ = self.close_tx.send(true);
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn start_core(dir: &std::path::Path) -> TestCore {
    let (router, out_rx) = DispatchHandle::new(SessionMap::new());
    let path = dir.join("core.sock");
    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    let serve_path = path.clone();
    let serve_router = router.clone();
    // wire-webui-sebas-agent-e2e：通道 server 委托复合 SessionBackend。
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(serve_router.clone()),
    );
    tokio::spawn(async move {
        let _ = server::serve(backend, serve_router, serve_path, SECRET.into(), close_rx).await;
    });
    for _ in 0..100 {
        if path.exists() {
            return TestCore {
                path,
                close_tx,
                _handle: router,
                _out_rx: out_rx,
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("core session channel socket never appeared");
}

/// 初始化全局状态引擎（本进程独占，仅第一次调用真实初始化）。
/// 返回 StateWriter 保持写者线程存活（借自进程级 OnceLock）。
/// 两个测试通过 `TEST_SERIAL` 串行执行，共享同一全局 engine/DB。
fn init_state_engine(dir: &std::path::Path) -> &'static StateWriter {
    use std::sync::OnceLock;
    static WRITER: OnceLock<StateWriter> = OnceLock::new();
    if sebas_dispatch::state_store::engine().is_none() {
        let path = dir.join("state.db");
        let writer = StateWriter::start(path).expect("state writer starts");
        let engine = Box::new(DbStateEngine::new(writer.handle().clone()));
        sebas_dispatch::state_store::init_engine(engine);
        let _ = WRITER.set(writer);
    }
    WRITER.get().expect("writer initialized")
}

/// 进程级串行锁：全局 engine 只能初始化一次（OnceLock），测试间共享
/// 同一 DB——必须串行执行避免状态串扰。
static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 4.2 验收：mutation 提交后，StateSubscribe 订阅端收到对应 scope 的
/// Changed 帧（快照帧之后）。
#[tokio::test]
async fn mutation_delivers_change_notification_on_subscription() {
    let _guard = TEST_SERIAL.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _writer = init_state_engine(dir.path());
    let core = start_core(dir.path()).await;

    let stream = UnixStream::connect(&core.path).await.unwrap();
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: SECRET.into(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();
    let mut ack = String::new();
    reader.read_line(&mut ack).await.unwrap();
    assert!(ack.contains("handshake"));

    let sub = serde_json::to_string(&CoreChannelRequest::StateSubscribe).unwrap();
    w.write_all(sub.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();

    // Frame 1: 全域快照（含四个域）。
    let mut line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("snapshot frame must arrive")
        .unwrap();
    assert!(n > 0);
    let frame: StateStreamFrame = serde_json::from_str(line.trim()).unwrap();
    match &frame {
        StateStreamFrame::Snapshot { domains } => {
            for domain in ["providers", "settings", "projects", "sessions"] {
                assert!(
                    domains.get(domain).is_some(),
                    "snapshot must include domain {domain}: {domains}"
                );
            }
        }
        other => panic!("first frame must be the snapshot, got {other:?}"),
    }

    // Mutation: settings（空对象 = 合法 CardConfig）+ projects（add）。
    let engine = sebas_dispatch::state_store::engine().expect("engine initialized");
    engine
        .save_settings(serde_json::json!({}))
        .await
        .expect("save settings");
    engine
        .add_project("/tmp/proj", "proj", 1700000000)
        .await
        .expect("add project");

    // 合并窗口 100ms：settings + projects 可能合并为一帧或两帧。断言两种
    // scope 都出现即可（合并语义允许合并，但 scope 集合必须完整）。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_settings = false;
    let mut saw_projects = false;
    loop {
        if saw_settings && saw_projects {
            break;
        }
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        line.clear();
        let read = tokio::time::timeout(timeout, reader.read_line(&mut line)).await;
        match read {
            Ok(Ok(n)) if n > 0 => {
                let frame: StateStreamFrame = serde_json::from_str(line.trim()).unwrap();
                match frame {
                    StateStreamFrame::Changed { scope } => {
                        if scope == "settings" {
                            saw_settings = true;
                        }
                        if scope == "projects" {
                            saw_projects = true;
                        }
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
    assert!(
        saw_settings,
        "settings mutation must deliver a Changed(settings) frame"
    );
    assert!(
        saw_projects,
        "projects mutation must deliver a Changed(projects) frame"
    );

    drop(w);
}

/// 5.3 通道代理：providers / aliases 经 `StateMutation` 写入状态库，
/// 随后的快照能读回（router admin 写路径的协议基础）。
#[tokio::test]
async fn providers_and_aliases_mutation_round_trip_over_channel() {
    let _guard = TEST_SERIAL.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    // engine 由前一个测试初始化（共享全局）；这里只需 core 通道服务。
    let _ = init_state_engine(dir.path());
    let core = start_core(dir.path()).await;

    // 一次性请求 helper：握手 + 发请求 + 读响应。
    async fn req_once(
        path: &std::path::Path,
        req: &serde_json::Value,
    ) -> serde_json::Value {
        let stream = UnixStream::connect(path).await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let hs = serde_json::to_string(&ChannelHandshake {
            secret: SECRET.into(),
        })
        .unwrap();
        w.write_all(hs.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.flush().await.unwrap();
        let mut ack = String::new();
        reader.read_line(&mut ack).await.unwrap();
        let body = serde_json::to_string(req).unwrap();
        w.write_all(body.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.flush().await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(line.trim()).unwrap()
    }

    // put provider（path 直连 core 的 StateMutation）。
    let resp = req_once(
        &core.path,
        &serde_json::json!({
            "cmd": "state_mutation",
            "domain": "providers",
            "payload": {
                "op": "put",
                "name": "anthropic",
                "item": {
                    "base_url_anthropic": "https://api.anthropic.com",
                    "api_key_env": "ANTHROPIC_API_KEY",
                }
            }
        }),
    )
    .await;
    assert_eq!(resp["cmd"], "state_mutation_ok", "put provider: {resp}");

    // put alias（引用已存在的 provider）。
    let resp = req_once(
        &core.path,
        &serde_json::json!({
            "cmd": "state_mutation",
            "domain": "aliases",
            "payload": {
                "op": "put",
                "alias": "my-claude",
                "entry": {"provider": "anthropic", "upstream_model": "claude-sonnet-4"}
            }
        }),
    )
    .await;
    assert_eq!(resp["cmd"], "state_mutation_ok", "put alias: {resp}");

    // 快照读回：providers 含 anthropic，aliases 随 PersistedState 带出。
    let resp = req_once(
        &core.path,
        &serde_json::json!({"cmd": "state_snapshot", "domain": "providers"}),
    )
    .await;
    assert_eq!(resp["cmd"], "state_snapshot");
    assert!(
        resp["payload"]["providers"]["anthropic"].is_object(),
        "providers snapshot must contain anthropic: {resp}"
    );
    assert_eq!(
        resp["payload"]["model_aliases"]["my-claude"]["provider"],
        "anthropic",
        "aliases must ride along in the providers snapshot: {resp}"
    );

    // delete alias → 快照不再含。
    let resp = req_once(
        &core.path,
        &serde_json::json!({
            "cmd": "state_mutation",
            "domain": "aliases",
            "payload": {"op": "delete", "alias": "my-claude"}
        }),
    )
    .await;
    assert_eq!(resp["cmd"], "state_mutation_ok", "delete alias: {resp}");
    let resp = req_once(
        &core.path,
        &serde_json::json!({"cmd": "state_snapshot", "domain": "providers"}),
    )
    .await;
    assert!(
        resp["payload"]["model_aliases"].get("my-claude").is_none(),
        "alias must be gone after delete: {resp}"
    );

    // delete provider → 墓碑生效 + 快照移除。
    let resp = req_once(
        &core.path,
        &serde_json::json!({
            "cmd": "state_mutation",
            "domain": "providers",
            "payload": {"op": "delete", "name": "anthropic"}
        }),
    )
    .await;
    assert_eq!(resp["cmd"], "state_mutation_ok", "delete provider: {resp}");
    let resp = req_once(
        &core.path,
        &serde_json::json!({"cmd": "state_snapshot", "domain": "providers"}),
    )
    .await;
    assert!(
        resp["payload"]["providers"].get("anthropic").is_none(),
        "provider must be gone: {resp}"
    );
    assert!(
        resp["payload"]["deleted"]
            .as_array()
            .is_some_and(|d| d.iter().any(|x| x == "anthropic")),
        "deleted tombstone must persist: {resp}"
    );
}