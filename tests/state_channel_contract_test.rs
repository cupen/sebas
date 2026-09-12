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
use std::collections::BTreeMap;
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
    /// add-fetch-models 1.2/1.3：providers 域条目 + 落盘次数计数。load/save
    /// 具备真实读写语义，「抓取不落盘」由保存计数与序列化快照逐字节比对断言。
    providers: std::sync::Mutex<BTreeMap<String, serde_json::Value>>,
    save_calls: std::sync::atomic::AtomicUsize,
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
        let mut state = sebas_dispatch::state_store::PersistedState::default();
        for (name, item) in self.inner.providers.lock().unwrap().iter() {
            state
                .providers
                .insert(name.clone(), item.as_object().cloned().unwrap_or_default());
        }
        state
    }
    async fn save_persisted_state(
        &self,
        _state: sebas_dispatch::state_store::PersistedState,
    ) -> anyhow::Result<()> {
        self.inner
            .save_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    async fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<(), String> {
        let mut g = self.inner.projects.lock().unwrap();
        let mut updated = false;
        for p in g.iter_mut() {
            if p.get("id").and_then(|v| v.as_str()) == Some(id) {
                p["default_agent"] = serde_json::json!(agent);
                updated = true;
            }
        }
        if updated {
            Ok(())
        } else {
            Err(format!("set_default_agent: project '{id}' 不存在"))
        }
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
/// multi_thread flavor：快照域含 router_activity，其计数经
/// provider_state::load() 合法 block_in_place——生产 server 只跑多线程
/// 运行时（block_on_engine 契约），current_thread 测试运行时会 panic。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
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

// ---- make-core-own-provider-data 1.2：provider / alias mutation 校验 ----
//
// 非法 payload（未知字段 / 类型错误）→ `Rejected` typed rejection（不静默
// 吞错），engine 状态未变。fake engine 的 save 是 no-op，「状态未变」由
// 「校验在 save 之前拒绝」的结构保证——channel 层断言的是 Rejected 回包。

/// providers 域：未知字段 → Rejected，cause 指名 provider 与字段。
#[tokio::test]
async fn provider_mutation_rejects_unknown_field() {
    let _inner = init_fake_state_store();
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "providers".into(),
            payload: serde_json::json!({
                "op": "put",
                "name": "evil",
                "item": {"preset": "deepseek", "not_a_field": 1}
            }),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(cause.contains("evil"), "cause names the provider: {cause}");
                assert!(cause.contains("not_a_field"), "cause names the field: {cause}");
            }
            other => panic!("expected Unavailable rejection, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
}

/// providers 域：类型错误（models 既非数组也非字符串）→ Rejected。
#[tokio::test]
async fn provider_mutation_rejects_wrong_typed_field() {
    let _inner = init_fake_state_store();
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "providers".into(),
            payload: serde_json::json!({
                "op": "put",
                "name": "typed",
                "item": {"models": {"oops": true}}
            }),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::Rejected { .. }),
        "wrong-typed models must be rejected, got {resp:?}"
    );
}

/// providers 域：合法条目仍被接受（校验不误伤）。
#[tokio::test]
async fn provider_mutation_accepts_valid_item() {
    let _inner = init_fake_state_store();
    // 有效 put 会真正调用 save_persisted_state——与观测 save 计数的
    // add-fetch-models 用例互斥（共享 fake engine 的计数是进程级）。
    let _g = PROVIDERS_STORE_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "providers".into(),
            payload: serde_json::json!({
                "op": "put",
                "name": "good",
                "item": {"preset": "deepseek", "api_key": "sk-x", "default_model": "deepseek-chat"}
            }),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::StateMutationOk),
        "valid item must be accepted, got {resp:?}"
    );
}

/// aliases 域：entry 缺 provider / 未知字段 → Rejected。
#[tokio::test]
async fn alias_mutation_rejects_invalid_entry() {
    let _inner = init_fake_state_store();
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    for payload in [
        serde_json::json!({"op": "put", "alias": "a1", "entry": {"upstream_model": "m"}}),
        serde_json::json!({"op": "put", "alias": "a2", "entry": {"provider": "p", "bogus": 1}}),
    ] {
        let resp = raw_request(
            &core.path,
            &CoreChannelRequest::StateMutation {
                domain: "aliases".into(),
                payload,
            },
        )
        .await
        .unwrap()
        .expect("answered");
        let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
        assert!(
            matches!(resp, CoreChannelResponse::Rejected { .. }),
            "invalid alias entry must be rejected, got {resp:?}"
        );
    }
}

/// settings 域：defaults ops（1.1）— set_defaults 缺 provider → Rejected。
#[tokio::test]
async fn settings_defaults_mutation_requires_provider() {
    let _inner = init_fake_state_store();
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "settings".into(),
            payload: serde_json::json!({"op": "set_defaults"}),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(cause.contains("provider"), "cause: {cause}");
            }
            other => panic!("expected Unavailable rejection, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
}

// ---- add-fetch-models 1.2/1.3：providers 域抓取 op ----
//
// 上游一律打向本进程内起的本地 mock（绝不外联）。fake engine 的 providers
// 段具备真实读写语义，「抓取不落盘」由 save 计数 + 快照逐字节比对断言。
//
// fake engine 是进程级共享单例：写 providers 段 / 观测 save 计数的用例必须
// 持同一把锁互斥（其余用例只碰独立键位的 projects/settings，不需要锁）。
static PROVIDERS_STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 本地 mock 上游：按场景回固定 `/models` 响应，并记录收到的 Authorization。
async fn start_models_mock(status: u16, body: &'static str) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let authz: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let authz_clone = authz.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let req = String::from_utf8_lossy(&buf).to_string();
            if let Some(line) = req.lines().find(|l| l.to_ascii_lowercase().starts_with("authorization")) {
                authz_clone.lock().unwrap().push(line.to_string());
            }
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    (format!("http://{addr}"), authz)
}

fn seed_provider(inner: &FakeStateInner, name: &str, item: serde_json::Value) {
    inner
        .providers
        .lock()
        .unwrap()
        .insert(name.to_string(), item);
}

/// 1.2 验收「成功返回 ids」：chat 槽位指向本地 mock，FetchModels 帧回
/// `Models { provider, models }`，密钥注入上游请求但绝不出现在响应里。
#[tokio::test]
async fn fetch_models_op_returns_ids_and_injects_key_upstream() {
    let (mock_url, authz) = start_models_mock(
        200,
        r#"{"object":"list","data":[{"id":"m-pro"},{"id":"m-flash"}]}"#,
    )
    .await;
    let inner = init_fake_state_store();
    let _g = PROVIDERS_STORE_LOCK.lock().await;
    seed_provider(
        &inner,
        "mocko",
        serde_json::json!({
            "name": "mocko",
            "base_url_openai_chat": format!("{mock_url}/v1"),
            "api_key": "sk-upstream-key-42"
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(&core.path, &CoreChannelRequest::FetchModels { provider: "mocko".into() })
        .await
        .unwrap()
        .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Models { provider, models } => {
            assert_eq!(provider, "mocko");
            assert_eq!(models, vec!["m-pro".to_string(), "m-flash".to_string()]);
        }
        other => panic!("expected Models, got {other:?}"),
    }
    // 上游请求带 Bearer key；响应里绝无 key 材料。
    let last = authz.lock().unwrap().last().cloned().unwrap_or_default();
    assert_eq!(
        last.split_once(':').map(|(h, v)| (h.trim().to_ascii_lowercase(), v.trim().to_string())),
        Some(("authorization".into(), "Bearer sk-upstream-key-42".into())),
        "resolved key must authenticate the upstream call"
    );

    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());
    let models = backend
        .fetch_provider_models("mocko")
        .await
        .expect("trait-level seam delivers the same op");
    let wire = serde_json::to_string(&models).unwrap();
    assert!(!wire.contains("sk-upstream-key-42"), "no key in ids: {wire}");
}

/// 1.2 验收「无 base url 回 typed rejection」：条目无任何槽位、也无 preset →
/// `Rejected`，cause 指名原因，且不发起任何上游请求（无 mock 可打）。
#[tokio::test]
async fn fetch_models_op_rejects_provider_without_base_url() {
    let inner = init_fake_state_store();
    let _g = PROVIDERS_STORE_LOCK.lock().await;
    seed_provider(&inner, "urlless", serde_json::json!({"name": "urlless"}));
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let resp = raw_request(&core.path, &CoreChannelRequest::FetchModels { provider: "urlless".into() })
        .await
        .unwrap()
        .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    match resp {
        CoreChannelResponse::Rejected { rejection } => match rejection {
            SessionRejection::Unavailable { cause } => {
                assert!(cause.contains("urlless"), "cause names the provider: {cause}");
                assert!(
                    cause.contains("base URL"),
                    "cause names the reason (no usable base url): {cause}"
                );
            }
            other => panic!("expected Unavailable rejection, got {other:?}"),
        },
        other => panic!("expected Rejected, got {other:?}"),
    }
}

/// 1.2 验收「上游错误回净化 reason」：401 + 含敏感字样的上游 body →
/// `Rejected`，cause 只含状态码/类别，不回显 body、不含密钥。
#[tokio::test]
async fn fetch_models_op_sanitizes_upstream_failure() {
    let (mock_url, _authz) = start_models_mock(
        401,
        r#"{"error":{"message":"invalid api key for sk-upstream-key-99","type":"auth"}}"#,
    )
    .await;
    let inner = init_fake_state_store();
    let _g = PROVIDERS_STORE_LOCK.lock().await;
    seed_provider(
        &inner,
        "mocko401",
        serde_json::json!({
            "name": "mocko401",
            "base_url_openai_chat": format!("{mock_url}/v1"),
            "api_key": "sk-upstream-key-99"
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;

    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());
    let err = backend
        .fetch_provider_models("mocko401")
        .await
        .expect_err("upstream 401 must be a typed failure");
    assert!(err.contains("401"), "cause names the status: {err}");
    assert!(!err.contains("sk-upstream-key-99"), "no key material: {err}");
    assert!(
        !err.contains("invalid api key"),
        "no upstream body echo: {err}"
    );
}

/// 1.3 验收：抓取前后 provider 快照逐字节一致，且引擎 save 计数为 0——
/// 抓取不改任何字段、不落盘；随后一次显式 put（普通编辑）才提交。
#[tokio::test]
async fn fetch_models_op_persists_nothing() {
    let (mock_url, _authz) = start_models_mock(
        200,
        r#"{"object":"list","data":[{"id":"m-pro"},{"id":"m-flash"}]}"#,
    )
    .await;
    let inner = init_fake_state_store();
    let _g = PROVIDERS_STORE_LOCK.lock().await;
    seed_provider(
        &inner,
        "frozen",
        serde_json::json!({
            "name": "frozen",
            "base_url_openai_chat": format!("{mock_url}/v1"),
            "api_key": "sk-keep",
            "default_model": "old-model",
            "models": ["old-model"]
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let core = start_core(dir.path()).await;
    let backend = CoreChannelBackend::new(core.path.clone(), SECRET.into());

    let before = serde_json::to_string(&backend.state_snapshot("providers").await.unwrap()).unwrap();
    let saves_before = inner.save_calls.load(std::sync::atomic::Ordering::SeqCst);

    let models = backend
        .fetch_provider_models("frozen")
        .await
        .expect("fetch succeeds");
    assert_eq!(models, vec!["m-pro".to_string(), "m-flash".to_string()]);

    let after = serde_json::to_string(&backend.state_snapshot("providers").await.unwrap()).unwrap();
    let saves_after = inner.save_calls.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(before, after, "provider snapshot byte-for-byte unchanged");
    assert_eq!(
        saves_before, saves_after,
        "the fetch op must never call save_persisted_state"
    );

    // 对照组：普通编辑（put）确实会落盘——证明 0-save 不是 harness 失效。
    let resp = raw_request(
        &core.path,
        &CoreChannelRequest::StateMutation {
            domain: "providers".into(),
            payload: serde_json::json!({
                "op": "put",
                "name": "frozen",
                "item": {"name": "frozen", "base_url_openai_chat": format!("{mock_url}/v1"),
                         "api_key": "sk-keep", "default_model": "m-pro", "models": ["old-model", "m-pro"]}
            }),
        },
    )
    .await
    .unwrap()
    .expect("answered");
    let resp: CoreChannelResponse = serde_json::from_str(&resp).unwrap();
    assert!(
        matches!(resp, CoreChannelResponse::StateMutationOk),
        "ordinary edit accepted, got {resp:?}"
    );
    assert!(
        inner.save_calls.load(std::sync::atomic::Ordering::SeqCst) > saves_after,
        "an explicit edit does save — the 0-save assertion above is real"
    );
}
