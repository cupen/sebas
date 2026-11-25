#![cfg(unix)]
// 串行化锁是文件级 env 隔离的意图（跨 await 持有正是目的），不是竞态。
#![allow(clippy::await_holding_lock)]
//! agent-task-center 接力 1：HTTP/WS 契约的确定性集成测试。
//!
//! 与 `task_center_integration_test.rs`（真实 `bd`、`#[ignore]`）互补：本文件
//! 用**假 `bd` 桩**（POSIX sh，canned 输出 + 调用日志）覆盖 spec 里必须逐条
//! 钉住的边界，且能在普通 `cargo test` 里确定性跑：
//!
//! - 「Claim is atomic」：409 点名当前持有者、同 actor 幂等 200、release 回队列；
//! - 「Assignee vocabulary」：未知 principal 400 且**不调** `bd`，agent kind 放行；
//! - 「Close is gated」：非 reviewer agent 403（`bd close` 未被调用）、reviewer
//!   agent 放行、人类恒放行；agent 身份 = config 种子 ∪ Settings store 行（P2-1）；
//! - 「Beads initialization is lazy」：首次访问才 `bd init`、已有 `.beads/`
//!   不重建、init 失败以 503 + stderr 原文内联呈现；**已确保项目不再 spawn**
//!   （P3-2）；
//! - 「Real-time board refresh」：多客户端订阅同一条泵（fan-out），单个客户端
//!   掉线不影响其余订阅者；
//! - 「Single-machine multi-agent scope」：refresh 把事件游标重基线到 journal
//!   head（不再重放陈旧事件）、journal 截断时用错误自带的 `head` 恢复（P2-2）、
//!   远端项目如实 400。

use axum::Extension;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// `SEBAS_BD_BIN` 是进程级环境变量：本文件的用例串行，避免桩路径互相踩。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

const SCRIPT: &str = r#"#!/bin/sh
DATA='__DATA__'
printf '%s\n' "$*" >> "$DATA/invocations.log"

# 全局 flag（-C/--actor 带值）先跳过，第一个其它词才是子命令。
sub=''
skip=''
for a in "$@"; do
  if [ -n "$skip" ]; then skip=''; continue; fi
  case "$a" in
    -C|--actor) skip=1 ;;
    --json|--no-color|--non-interactive) ;;
    *) sub="$a"; break ;;
  esac
done

emit() {
  [ -f "$DATA/$1" ] && cat "$DATA/$1"
  return 0
}

# 预置 `<sub>.err` 即让该子命令以非零退出（stderr 原文透传）。
fail_if_seeded() {
  if [ -f "$DATA/$1.err" ]; then cat "$DATA/$1.err" >&2; exit 1; fi
}

case "$sub" in
  init)
    fail_if_seeded init
    mkdir -p ./.beads
    exit 0
    ;;
  config) exit 0 ;;
  list) fail_if_seeded list; emit list.json; exit 0 ;;
  show) fail_if_seeded show; emit show.json; exit 0 ;;
  create) fail_if_seeded create; emit create.json; exit 0 ;;
  update)
    case "$*" in
      *--claim*) fail_if_seeded claim; emit claim.json ;;
      *--add-label*) fail_if_seeded review; emit review.json ;;
      *) echo "unexpected update: $*" >&2; exit 2 ;;
    esac
    exit 0
    ;;
  unclaim) fail_if_seeded unclaim; emit unclaim.json; exit 0 ;;
  assign) fail_if_seeded assign; emit assign.json; exit 0 ;;
  close) fail_if_seeded close; emit close.json; exit 0 ;;
  comment) fail_if_seeded comment; emit comment.json; exit 0 ;;
  events)
    fail_if_seeded events
    since=0
    prev=''
    for a in "$@"; do
      if [ "$prev" = '--since' ]; then since="$a"; fi
      prev="$a"
    done
    if [ -f "$DATA/events.ndjson" ]; then
      while IFS= read -r line; do
        [ -z "$line" ] && continue
        seq=$(printf '%s' "$line" | sed -n 's/.*"seq":\([0-9][0-9]*\).*/\1/p')
        if [ -n "$seq" ] && [ "$seq" -gt "$since" ]; then printf '%s\n' "$line"; fi
      done < "$DATA/events.ndjson"
    fi
    exit 0
    ;;
  *) echo "unknown command: $sub" >&2; exit 2 ;;
esac
"#;

/// 假 `bd`：脚本 + 数据目录（canned 输出 + 调用日志）。
struct FakeBd {
    root: tempfile::TempDir,
}

impl FakeBd {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let script = SCRIPT.replace("__DATA__", &data.display().to_string());
        write_script(&root.path().join("bd"), &script);
        Self { root }
    }

    fn bin(&self) -> PathBuf {
        self.root.path().join("bd")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    /// 预置一次命令的 stdout（`name` 如 `list.json`）或失败 stderr
    /// （`name` 以 `.err` 结尾即让该子命令非零退出）。
    fn seed(&self, name: &str, content: &str) {
        std::fs::write(self.data().join(name), content).unwrap();
    }

    /// 预置 events journal（每行一条 NDJSON）。
    fn seed_events(&self, lines: &[String]) {
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        self.seed("events.ndjson", &body);
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.data().join("invocations.log")).unwrap_or_default()
    }

    fn invocation_count(&self, needle: &str) -> usize {
        self.log()
            .lines()
            .filter(|line| line.contains(needle))
            .count()
    }
}

fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

struct App {
    base: String,
    http: reqwest::Client,
    project_id: String,
    project_dir: PathBuf,
    _root: tempfile::TempDir,
}

/// 装配一个指向假 `bd` 的 WebUI（项目注册表用 `FakeBackend` 的内存域，
/// 不触进程级状态库）。
///
/// `store_agents` 模拟 Settings → Agents 里建的 agent kind（`agents` 域的
/// store 行，P2-1：config 种子之外的 agent 身份来源）。
async fn spawn_app_with(
    bd: &FakeBd,
    agent_kinds: &[&str],
    store_agents: &[&str],
    identity: Option<sebas_webui::auth::Identity>,
    node_id: &str,
) -> App {
    // 必须在构造 WebUiState（`BeadsClient::new()` 读 env）之前落地。
    unsafe { std::env::set_var(sebas_webui::beads_client::BD_BIN_ENV, bd.bin()) };

    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("task-center-project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let backend = Arc::new(sebas_webui::session_backend::FakeBackend::new());
    let project_id = backend.seed_project(node_id, project_dir.to_str().unwrap());
    if !store_agents.is_empty() {
        let rows: Vec<Value> = store_agents.iter().map(|id| json!({ "id": id })).collect();
        backend.set_state_domain("agents", Some(json!({ "agents": rows })));
    }
    let backend: Arc<dyn sebas_webui::SessionBackend> = backend;

    let kinds = Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
        agent_kinds
            .iter()
            .map(|slug| sebas_webui::agent_kinds::AgentKindSource {
                slug: (*slug).to_string(),
                command: vec![(*slug).to_string()],
                driver: "claude".to_string(),
                display: None,
            })
            .collect(),
    ));

    let mut app = sebas_webui::server::build_router_with_workspace_root(
        backend,
        sebas_webui::models::RouterInfo::default(),
        sebas_feishu::cards::CardConfig::default(),
        kinds,
        Arc::new(sebas_webui::auth::AuthHandle::disabled()),
        root.path().to_path_buf(),
    );
    if let Some(identity) = identity {
        // 鉴权关闭时没有身份解析中间件；测试直接注入调用者身份。
        app = app.layer(Extension(identity));
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    App {
        base: format!("http://{addr}"),
        http: reqwest::Client::new(),
        project_id,
        project_dir,
        _root: root,
    }
}

async fn spawn_app(
    bd: &FakeBd,
    agent_kinds: &[&str],
    identity: Option<sebas_webui::auth::Identity>,
) -> App {
    spawn_app_with(
        bd,
        agent_kinds,
        &[],
        identity,
        sebas_webui::projects::LOCAL_NODE_ID,
    )
    .await
}

fn identity(username: &str) -> sebas_webui::auth::Identity {
    sebas_webui::auth::Identity {
        user_id: 1,
        username: username.to_string(),
        role: sebas_webui::rbac::Role::Member,
    }
}

impl App {
    fn tasks_url(&self) -> String {
        format!("{}/api/projects/{}/tasks", self.base, self.project_id)
    }
}

async fn http_json(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let mut request = http.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("http request");
    let status = response.status();
    let value = response.json::<Value>().await.unwrap_or(Value::Null);
    (status, value)
}

async fn next_frame(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for a WebSocket frame")
            .expect("websocket stream ended")
            .expect("websocket error");
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).expect("frame must be JSON");
        }
    }
}

async fn watch(
    base: &str,
    project_id: &str,
    id: u64,
) -> impl SinkExt<Message>
+ StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
+ Unpin {
    let url = format!("{}/ws", base.replacen("http://", "ws://", 1))
        .into_client_request()
        .unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");
    ws.send(Message::Text(
        json!({
            "id": id,
            "method": "tasks.watch",
            "params": { "project_id": project_id },
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    ws
}

fn event_line(seq: i64) -> String {
    json!({
        "seq": seq,
        "ts": format!("2026-11-24T12:00:0{seq}Z"),
        "op": "update",
        "issue_id": format!("tsk-{seq}"),
        "issue": {
            "id": format!("tsk-{seq}"),
            "title": format!("task {seq}"),
            "status": "open",
        },
    })
    .to_string()
}

fn issue(status: &str, assignee: Option<&str>, reviewer: Option<&str>) -> String {
    let mut value = json!({
        "id": "tsk-1",
        "title": "T",
        "status": status,
        "labels": ["in-review"],
    });
    if let Some(assignee) = assignee {
        value["assignee"] = json!(assignee);
    }
    if let Some(reviewer) = reviewer {
        value["metadata"] = json!({ "reviewer": reviewer });
    }
    format!("[{value}]")
}

fn app_bd_log(bd: &FakeBd) -> String {
    bd.log()
}

// ---------------------------------------------------------------------------
// Claim is atomic and delegated to beads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_race_returns_409_naming_the_current_holder() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "claim.err",
        "Error updating tsk-1: issue already claimed: already assigned to \"claude\" — coordinate\n",
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/claim", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "already_claimed", "{body}");
    assert_eq!(body["holder"], "claude", "409 必须点名当前持有者: {body}");
}

#[tokio::test]
async fn claim_is_idempotent_for_the_same_actor() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "claim.json",
        r#"[{"id":"tsk-1","title":"T","status":"in_progress","assignee":"admin"}]"#,
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/claim", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["status"], "in_progress", "{body}");
    assert_eq!(body["assignee"], "admin", "{body}");
    assert!(
        app_bd_log(&bd).contains("--actor admin update tsk-1 --claim"),
        "认领必须以调用者为 bd actor: {}",
        app_bd_log(&bd)
    );
}

#[tokio::test]
async fn release_returns_the_task_to_the_public_queue() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "unclaim.json",
        r#"[{"id":"tsk-1","title":"T","status":"open"}]"#,
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/release", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["status"], "open", "释放必须回到 open: {body}");
    assert!(
        app_bd_log(&bd).contains("unclaim tsk-1"),
        "{:?}",
        app_bd_log(&bd)
    );
}

/// P3-1：assign / unclaim 的撞车措辞（`held by "X"` / `is held by X;`）也是
/// 409 + 点名持有者，而不是掉进 500。
#[tokio::test]
async fn assign_and_release_conflicts_surface_as_409_naming_the_holder() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "assign.err",
        "cannot reassign tsk-1: held by \"claude\" (in_progress)\n",
    );
    bd.seed(
        "unclaim.err",
        "issue claimed by a different actor: tsk-1 is held by claude; reclaim or use --force\n",
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/assign", app.tasks_url()),
        Some(json!({ "assignee": "claude" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{body}");
    assert_eq!(body["holder"], "claude", "{body}");

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/release", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{body}");
    assert_eq!(body["holder"], "claude", "{body}");
}

// ---------------------------------------------------------------------------
// Assignee vocabulary is validated at the WebUI boundary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_assignee_is_rejected_without_invoking_bd() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/assign", app.tasks_url()),
        Some(json!({ "assignee": "nobody" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|message| message.contains("nobody")),
        "400 必须点名未知 principal: {body}"
    );
    assert!(
        app_bd_log(&bd).is_empty(),
        "词表拒绝必须发生在调 bd 之前: {}",
        app_bd_log(&bd)
    );
}

#[tokio::test]
async fn configured_agent_kind_is_a_valid_assignee() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "assign.json",
        r#"{"id":"tsk-1","title":"T","status":"open","assignee":"claude"}"#,
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/assign", app.tasks_url()),
        Some(json!({ "assignee": "claude" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["assignee"], "claude", "{body}");
    assert!(
        app_bd_log(&bd).contains("assign tsk-1 claude"),
        "{:?}",
        app_bd_log(&bd)
    );
}

/// P2-1：词表与评审闸门共用 agent 身份集合——Settings store 里建的 agent
/// kind（不在 config 种子内）同样是合法 assignee。
#[tokio::test]
async fn store_agent_row_is_a_valid_assignee() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "assign.json",
        r#"{"id":"tsk-1","title":"T","status":"open","assignee":"codex"}"#,
    );
    let app = spawn_app_with(
        &bd,
        &["claude"],
        &["codex"],
        Some(identity("admin")),
        sebas_webui::projects::LOCAL_NODE_ID,
    )
    .await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/assign", app.tasks_url()),
        Some(json!({ "assignee": "codex" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["assignee"], "codex", "{body}");
}

// ---------------------------------------------------------------------------
// Close is gated to humans and the designated reviewer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_reviewer_agent_is_refused_and_bd_close_never_runs() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "show.json",
        &issue("in_progress", Some("claude"), Some("claude")),
    );
    let app = spawn_app(&bd, &["claude", "codex"], Some(identity("codex"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/close", app.tasks_url()),
        Some(json!({ "reason": "reviewed" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN, "{body}");
    assert!(
        !app_bd_log(&bd).lines().any(|line| line.contains(" close ")),
        "403 必须发生在 bd close 之前: {}",
        app_bd_log(&bd)
    );
}

/// P2-1：**只**在 Settings store 里出现的 agent kind 也必须走闸门——曾只查
/// config 种子，store 行被当成人类、可关任意任务。
#[tokio::test]
async fn store_only_agent_kind_is_gated_like_a_configured_one() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "show.json",
        &issue("in_progress", Some("claude"), Some("claude")),
    );
    let app = spawn_app_with(
        &bd,
        &["claude"],
        &["codex"],
        Some(identity("codex")),
        sebas_webui::projects::LOCAL_NODE_ID,
    )
    .await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/close", app.tasks_url()),
        Some(json!({ "reason": "reviewed" })),
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::FORBIDDEN,
        "store 行 agent 不是 reviewer 时必须 403: {body}"
    );
    assert!(
        !app_bd_log(&bd).lines().any(|line| line.contains(" close ")),
        "403 必须发生在 bd close 之前: {}",
        app_bd_log(&bd)
    );
}

#[tokio::test]
async fn reviewer_agent_may_close() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "show.json",
        &issue("in_progress", Some("claude"), Some("claude")),
    );
    bd.seed(
        "close.json",
        r#"[{"id":"tsk-1","title":"T","status":"closed","close_reason":"reviewed"}]"#,
    );
    let app = spawn_app(&bd, &["claude", "codex"], Some(identity("claude"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/close", app.tasks_url()),
        Some(json!({ "reason": "reviewed" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["status"], "closed", "{body}");
    assert!(
        app_bd_log(&bd).contains("close tsk-1 --reason reviewed"),
        "{:?}",
        app_bd_log(&bd)
    );
}

#[tokio::test]
async fn human_operator_always_may_close() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "show.json",
        &issue("in_progress", Some("claude"), Some("claude")),
    );
    bd.seed(
        "close.json",
        r#"[{"id":"tsk-1","title":"T","status":"closed","close_reason":"cancelled"}]"#,
    );
    // alice 不在 agent 目录里 = 人类：绕过 reviewer 限制。
    let app = spawn_app(&bd, &["claude", "codex"], Some(identity("alice"))).await;

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/tsk-1/close", app.tasks_url()),
        Some(json!({ "reason": "cancelled" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["status"], "closed", "{body}");
    assert_eq!(body["close_reason"], "cancelled", "{body}");
}

// ---------------------------------------------------------------------------
// Beads repository initialization is lazy and automatic
// ---------------------------------------------------------------------------

/// P3-2：惰性初始化按项目**只跑一次**——后续端点访问不再 spawn
/// `bd init` / `bd config set`（每次请求都 spawn 是热路径上的无谓进程）。
#[tokio::test]
async fn first_visit_initializes_and_later_visits_reuse_the_cache() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed("list.json", "[]");
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(&app.http, reqwest::Method::GET, app.tasks_url(), None).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["tasks"], json!([]));
    assert!(
        app.project_dir.join(".beads").exists(),
        "首次访问必须初始化 .beads/"
    );
    assert_eq!(bd.invocation_count("init --prefix"), 1, "{}", bd.log());
    assert_eq!(
        bd.invocation_count("config set events-journal true"),
        1,
        "{}",
        bd.log()
    );

    for _ in 0..2 {
        let (status, _) = http_json(&app.http, reqwest::Method::GET, app.tasks_url(), None).await;
        assert_eq!(status, reqwest::StatusCode::OK);
    }
    assert_eq!(
        bd.invocation_count("init --prefix"),
        1,
        "后续访问不得重复 init: {}",
        bd.log()
    );
    assert_eq!(
        bd.invocation_count("config set events-journal true"),
        1,
        "已确保的项目不得每次访问都 spawn bd config（P3-2）: {}",
        bd.log()
    );
}

#[tokio::test]
async fn existing_beads_repository_is_preserved() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed("list.json", "[]");
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;
    std::fs::create_dir_all(app.project_dir.join(".beads")).unwrap();

    let (status, _) = http_json(&app.http, reqwest::Method::GET, app.tasks_url(), None).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(
        bd.invocation_count("init --prefix"),
        0,
        "已有 .beads/ 时不得重建: {}",
        bd.log()
    );
    assert_eq!(
        bd.invocation_count("config set events-journal true"),
        1,
        "{}",
        bd.log()
    );
}

#[tokio::test]
async fn init_failure_surfaces_inline_as_503_with_stderr() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "init.err",
        "bd: cannot initialize repository: permission denied\n",
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let (status, body) = http_json(&app.http, reqwest::Method::GET, app.tasks_url(), None).await;
    assert_eq!(status, reqwest::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "beads_init_failed", "{body}");
    assert!(
        body["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("permission denied")),
        "503 必须带 bd stderr 原文: {body}"
    );
    assert!(
        !app.project_dir.join(".beads").exists(),
        "init 失败不得留下半成品仓库"
    );
    assert_eq!(
        bd.invocation_count(" list"),
        0,
        "初始化失败不得继续列任务: {}",
        bd.log()
    );
}

// ---------------------------------------------------------------------------
// Real-time board refresh via WebSocket push
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_events_fan_out_to_every_subscribed_client() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed("list.json", "[]");
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let mut first = watch(&app.base, &app.project_id, 1).await;
    let ack = next_frame(&mut first).await;
    assert_eq!(ack["result"]["status"], "watching", "{ack}");
    let mut second = watch(&app.base, &app.project_id, 2).await;
    let ack = next_frame(&mut second).await;
    assert_eq!(ack["result"]["status"], "watching", "{ack}");

    bd.seed_events(&[event_line(1)]);
    for ws in [&mut first, &mut second] {
        let event = next_frame(ws).await;
        assert_eq!(event["method"], "task_updated", "{event}");
        assert_eq!(event["params"]["seq"], 1, "{event}");
        assert_eq!(event["params"]["issue"]["id"], "tsk-1", "{event}");
    }

    bd.seed_events(&[event_line(1), event_line(2)]);
    for ws in [&mut first, &mut second] {
        let event = next_frame(ws).await;
        assert_eq!(
            event["params"]["seq"], 2,
            "同一事件必须到达每个订阅者: {event}"
        );
    }

    // 一个客户端掉线不影响其余订阅者。
    drop(first);
    bd.seed_events(&[event_line(1), event_line(2), event_line(3)]);
    let event = next_frame(&mut second).await;
    assert_eq!(
        event["params"]["seq"], 3,
        "剩余订阅者必须继续收到增量: {event}"
    );
}

#[tokio::test]
async fn refresh_rebaselines_the_cursor_without_stale_replay() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "list.json",
        r#"[{"id":"tsk-1","title":"T","status":"open"}]"#,
    );
    bd.seed_events(&[event_line(1), event_line(2)]);
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let mut ws = watch(&app.base, &app.project_id, 1).await;
    let ack = next_frame(&mut ws).await;
    assert_eq!(ack["result"]["status"], "watching", "{ack}");
    assert_eq!(next_frame(&mut ws).await["params"]["seq"], 1);
    assert_eq!(next_frame(&mut ws).await["params"]["seq"], 2);

    // 模拟 `bd dolt pull`：journal 出现远端事件（seq 3）。
    bd.seed_events(&[event_line(1), event_line(2), event_line(3)]);
    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/refresh", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["tasks"].as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(
        body["last_seq"], 3,
        "refresh 必须把游标钉到 journal head: {body}"
    );

    // 重基线之后只投递新事件：seq ≤ 3 不再重放。
    bd.seed_events(&[event_line(1), event_line(2), event_line(3), event_line(4)]);
    let event = next_frame(&mut ws).await;
    assert_eq!(
        event["params"]["seq"], 4,
        "重基线后不得重放陈旧事件: {event}"
    );
}

/// P2-2：journal 已截断时 `--since 0` 必失败；refresh 用错误自带的 `head`
/// 重新基线并**照常返回任务列表**（曾恒 409，恢复入口不可用）。
#[tokio::test]
async fn refresh_recovers_from_a_truncated_journal_using_head() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    bd.seed(
        "list.json",
        r#"[{"id":"tsk-1","title":"T","status":"open"}]"#,
    );
    bd.seed_events(&[event_line(1), event_line(2), event_line(3)]);
    bd.seed(
        "events.err",
        r#"{"error":"events_journal_truncated","since":0,"floor":7,"head":20,"code":"events_journal_truncated"}
"#,
    );
    let app = spawn_app(&bd, &["claude"], Some(identity("admin"))).await;

    let mut ws = watch(&app.base, &app.project_id, 1).await;
    let ack = next_frame(&mut ws).await;
    assert_eq!(ack["result"]["status"], "watching", "{ack}");
    // 等泵先观察到截断（否则 refresh 与泵的基线顺序不可控）。
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        bd.invocation_count("events tail --since 0") >= 1,
        "泵必须已经轮询过: {}",
        bd.log()
    );

    let (status, body) = http_json(
        &app.http,
        reqwest::Method::POST,
        format!("{}/refresh", app.tasks_url()),
        None,
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "journal 截断时 refresh 必须照常返回任务列表: {body}"
    );
    assert_eq!(body["tasks"].as_array().map(Vec::len), Some(1), "{body}");
    assert_eq!(
        body["last_seq"], 20,
        "恢复基线必须是 journal head（不是 floor）: {body}"
    );

    // 截断消失后只投递新事件：无论游标停在 head(20) 还是 floor-1(6)，
    // 陈旧的 1..3 都不得重放。
    std::fs::remove_file(bd.data().join("events.err")).unwrap();
    bd.seed_events(&[event_line(1), event_line(2), event_line(3), event_line(21)]);
    let event = next_frame(&mut ws).await;
    assert_eq!(
        event["params"]["seq"], 21,
        "恢复后必须从 head 继续、不重放: {event}"
    );
}

// ---------------------------------------------------------------------------
// Single-machine multi-agent scope (v1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remote_project_is_rejected_as_out_of_scope() {
    let _guard = env_guard();
    let bd = FakeBd::new();
    let app = spawn_app_with(&bd, &["claude"], &[], None, "remote-node-1").await;

    let (status, body) = http_json(&app.http, reqwest::Method::GET, app.tasks_url(), None).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|message| message.contains("远程")),
        "远端项目必须如实说明 v1 单机范围: {body}"
    );
    assert!(
        app_bd_log(&bd).is_empty(),
        "远端项目不得在本机 spawn bd: {}",
        app_bd_log(&bd)
    );
}
