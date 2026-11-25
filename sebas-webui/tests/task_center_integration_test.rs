//! agent-task-center 3.10 / 4.2 / 4.3 集成测试（`#[ignore]`）。
//!
//! 旅程：注册项目 → 首次任务面板（惰性 `bd init`）→ 建任务 → 认领 → 进入
//! 评审 → 关闭 → 评论 → 手动 Refresh；全程经真实 HTTP，并由一条 WebSocket
//! 客户端观察 `task_updated` 推送。
//!
//! 需要本机 `bd` ≥ 1.3.4（`SEBAS_BD_BIN` 未设置时走 `PATH`）：
//!
//! ```text
//! cargo test -p sebas-webui --test task_center_integration_test -- --ignored --test-threads=1
//! ```
//!
//! 沙箱纪律：`bd` 的全局状态（`$HOME/.beads`、`$HOME/.config/bd`、
//! `$HOME/.dolt`）与项目库全部钉进一次性目录，绝不碰操作员真实主目录。

use futures_util::{SinkExt, StreamExt};
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_feishu::cards::CardConfig;
use sebas_webui::models::RouterInfo;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

mod common;

/// bd 可执行文件：显式覆盖优先，否则 `PATH` 上的 `bd`。
fn bd_bin() -> std::path::PathBuf {
    std::env::var_os(sebas_webui::beads_client::BD_BIN_ENV)
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| "bd".into())
}

fn bd_available() -> bool {
    std::process::Command::new(bd_bin())
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn next_frame(
    ws: &mut (
             impl StreamExt<
        Item = Result<
            tokio_tungstenite::tungstenite::Message,
            tokio_tungstenite::tungstenite::Error,
        >,
    > + Unpin
         ),
) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for a WebSocket frame")
            .expect("websocket stream ended")
            .expect("websocket error");
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            return serde_json::from_str(&text).expect("frame must be JSON");
        }
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a real bd (>= 1.3.4); run with --ignored --test-threads=1"]
async fn full_task_lifecycle_over_http_and_ws() {
    // bd 全局状态落点钉进一次性目录（本文件只有一个测试，进程级 env 修改
    // 不会串味；测试进程结束即回收）。**先钉 HOME**，再探测 bd——任何 bd
    // 调用都不得落在操作员的真实主目录。
    let home = tempfile::tempdir().unwrap();
    // SAFETY: 单测试二进制，且在 spawn 任何 bd 之前设置。
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("USER", "task-center-test");
        std::env::remove_var("GIT_AUTHOR_NAME");
    }

    if !bd_available() {
        panic!(
            "任务中心集成测试需要 bd >= 1.3.4：把 {} 指向可执行文件或加入 PATH",
            sebas_webui::beads_client::BD_BIN_ENV
        );
    }

    common::init();
    common::reset_projects();

    // 项目目录必须落在 workspace root 内（add-workspace-root 2.2）。
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("task-center-project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let map = SessionMap::new();
    let (dispatch, _rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(dispatch.clone()),
    );
    let app = sebas_webui::server::build_router_with_workspace_root(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
            vec![sebas_webui::agent_kinds::AgentKindSource {
                slug: "claude".into(),
                command: vec!["claude".into()],
                driver: "claude".into(),
                display: None,
            }],
        )),
        Arc::new(sebas_webui::auth::AuthHandle::disabled()),
        root.path().to_path_buf(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // 注册项目。
    let (status, project) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{base}/api/projects"),
        Some(json!({ "path": project_dir.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "register: {project}");
    let project_id = project["id"].as_str().expect("project id").to_string();
    let tasks_url = format!("{base}/api/projects/{project_id}/tasks");

    // 1) 首次访问：惰性 `bd init`，空列表。
    let (status, body) = http_json(&http, reqwest::Method::GET, tasks_url.clone(), None).await;
    assert_eq!(status, reqwest::StatusCode::OK, "list: {body}");
    assert_eq!(body["tasks"], json!([]));
    assert!(
        project_dir.join(".beads").exists(),
        "首次访问必须初始化 .beads/（bd init 的落点）"
    );

    // 2) WebSocket 订阅该项目。
    let ws_url = format!("{}/ws", base.replacen("http://", "ws://", 1))
        .into_client_request()
        .unwrap();
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .expect("ws connect");
    let (mut writer, mut reader) = ws_stream.split();
    writer
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "id": 1,
                "method": "tasks.watch",
                "params": { "project_id": project_id },
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let reply = next_frame(&mut reader).await;
    assert_eq!(reply["id"], 1, "watch reply: {reply}");
    assert_eq!(
        reply["result"]["status"], "watching",
        "watch reply: {reply}"
    );

    // 3) 建任务（reviewer 落 metadata）。
    let (status, created) = http_json(
        &http,
        reqwest::Method::POST,
        tasks_url.clone(),
        Some(json!({
            "title": "Ship the task center",
            "description": "relay 1 验收",
            "reviewer": "claude",
        })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "create: {created}");
    let task_id = created["id"].as_str().expect("task id").to_string();
    assert_eq!(created["status"], "open");
    assert_eq!(created["metadata"]["reviewer"], "claude");

    // 4) WS 推送 `task_updated`：完整 issue 快照（op = create）。
    let event = next_frame(&mut reader).await;
    assert_eq!(event["method"], "task_updated", "event: {event}");
    assert_eq!(event["params"]["project_id"], project_id.as_str());
    assert_eq!(event["params"]["op"], "create");
    assert_eq!(event["params"]["issue"]["id"], task_id.as_str());

    // 5) 认领（beads 原子 claim）。
    let (status, claimed) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{tasks_url}/{task_id}/claim"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "claim: {claimed}");
    assert_eq!(claimed["status"], "in_progress");
    assert!(
        claimed["assignee"].as_str().is_some_and(|a| !a.is_empty()),
        "claim 必须记录 assignee: {claimed}"
    );

    // 6) 进入评审：`in-review` label 出现在后续读取里。
    let (status, reviewed) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{tasks_url}/{task_id}/review"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "review: {reviewed}");
    assert!(
        reviewed["labels"]
            .as_array()
            .is_some_and(|labels| labels.iter().any(|label| label == "in-review")),
        "review 必须打上 in-review: {reviewed}"
    );

    // 7) 关闭（人类放行评审闸门；`--force` 穿过 actor≠assignee 自检）。
    let (status, closed) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{tasks_url}/{task_id}/close"),
        Some(json!({ "reason": "reviewed" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "close: {closed}");
    assert_eq!(closed["status"], "closed");
    assert_eq!(closed["close_reason"], "reviewed");

    // 8) 评论：作者是调用者（auth 关闭时是 bd 的默认 actor）。
    let (status, comment) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{tasks_url}/{task_id}/comment"),
        Some(json!({ "text": "LGTM" })),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CREATED, "comment: {comment}");
    assert_eq!(comment["text"], "LGTM");

    // 9) show：完整任务（含评论）。
    let (status, detail) = http_json(
        &http,
        reqwest::Method::GET,
        format!("{tasks_url}/{task_id}"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "show: {detail}");
    assert_eq!(detail["comments"].as_array().map(Vec::len), Some(1));
    assert_eq!(detail["comments"][0]["text"], "LGTM");

    // 10) 手动 Refresh：全量重载 + 事件游标钉到 journal head。
    let (status, refreshed) = http_json(
        &http,
        reqwest::Method::POST,
        format!("{tasks_url}/refresh"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "refresh: {refreshed}");
    assert_eq!(refreshed["tasks"].as_array().map(Vec::len), Some(1));
    assert!(
        refreshed["last_seq"].as_i64().unwrap_or(0) >= 1,
        "refresh 必须把 last_seq 钉到 journal head: {refreshed}"
    );

    // 11) 词表：未知 assignee 400，且不落任何 issue。
    let (status, rejected) = http_json(
        &http,
        reqwest::Method::POST,
        tasks_url.clone(),
        Some(json!({ "title": "should be rejected", "assignee": "nobody" })),
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::BAD_REQUEST,
        "vocabulary: {rejected}"
    );
    assert!(
        rejected["error"]
            .as_str()
            .is_some_and(|message| message.contains("nobody")),
        "400 必须点名未知 principal: {rejected}"
    );
    let (_, body) = http_json(&http, reqwest::Method::GET, tasks_url.clone(), None).await;
    assert_eq!(
        body["tasks"].as_array().map(Vec::len),
        Some(1),
        "被拒请求不得创建 issue: {body}"
    );

    // 12) 未知任务的 404 与未知项目的 404 都是类型化错误。
    let (status, _) = http_json(
        &http,
        reqwest::Method::GET,
        format!("{tasks_url}/tsk-does-not-exist"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    let (status, _) = http_json(
        &http,
        reqwest::Method::GET,
        format!("{base}/api/projects/no-such-project/tasks"),
        None,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let _ = writer
        .send(tokio_tungstenite::tungstenite::Message::Close(None))
        .await;
}
