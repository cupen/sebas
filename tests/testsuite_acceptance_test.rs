//! Acceptance suite (testsuite-acceptance): multi-step, journey-level cases that
//! cross several capabilities over real process boundaries. Sandbox rules are
//! the same as `testsuite_e2e_test` (support::Sandbox): everything inside a
//! throwaway dir, probed ports, no operator instance touched.
//!
//! Opt-in only: `cargo test --test testsuite_acceptance_test -- --ignored`
//! or `invoke testsuite-acceptance` (`--case <name>` filters). Coverage accounting for
//! these cases lives in `tests/acceptance/COVERAGE.md`.

use std::sync::Arc;
use std::time::Duration;

mod support;

use support::{Sandbox, http_client, post_json, spawn_stub_upstream, wait_for, wait_router_addr};

const TURN: Duration = Duration::from_secs(30);
const STARTUP: Duration = Duration::from_secs(30);

async fn create_session(cli: &reqwest::Client, sb: &Sandbox, body: serde_json::Value) -> String {
    let (status, resp) = post_json(cli, &format!("{}/api/sessions", sb.webui_url()), body)
        .await
        .expect("create session");
    assert_eq!(status, 201, "create session: {resp}");
    resp["key"].as_str().expect("session key").to_string()
}

/// Poll a session detail until its turn reaches Done; returns the
/// concatenated transcript text.
async fn wait_turn_done(cli: &reqwest::Client, sb: &Sandbox, key: &str) -> String {
    let url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();
    let detail = wait_for("session turn to reach Done", TURN, &hint, move || {
        let cli = cli.clone();
        let url = url.clone();
        Box::pin(async move {
            let v = cli
                .get(&url)
                .send()
                .await
                .ok()?
                .json::<serde_json::Value>()
                .await
                .ok()?;
            let done = v["status_slug"].as_str() == Some("done")
                || v["status"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("done"));
            done.then_some(v)
        })
    })
    .await;
    detail["entries"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["content"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Core session-management journey (lifecycle + persistence + restart
/// recovery): create → turn → follow-up message → core restart → mapping
/// restored from the state file.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn session_lifecycle_journey() {
    let sb = Sandbox::new("acceptance", "lifecycle");
    let cli = http_client();
    let mut core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // 1) create + first turn
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({ "prompt": "hello", "agent": "claude" }),
    )
    .await;
    let first = wait_turn_done(&cli, &sb, &key).await;
    assert!(first.contains("hello"), "first turn reply: {first:?}");

    // 2) follow-up message on the same session (continue)
    let (msg_status, msg_resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("send follow-up");
    assert_eq!(msg_status, 200, "follow-up message: {msg_resp}");
    let second = wait_turn_done(&cli, &sb, &key).await;
    assert!(!second.is_empty(), "second turn must produce output");

    // 3) Shutdown the core, then bring it back: the session mapping must
    //    survive via the persisted state (restart-recovery semantics).
    //    Restoring requires a GRACEFUL exit (the state dump happens on
    //    shutdown); Windows has no portable graceful signal for a child, so
    //    the restore segment is unix-gated like the graceful-exit coverage.
    #[cfg(unix)]
    let hint = sb.path.clone();
    #[cfg(unix)]
    {
        let pid = core.id().expect("core pid") as libc::pid_t;
        let exit: Arc<tokio::sync::Mutex<Option<String>>> = Arc::default();
        {
            let exit = exit.clone();
            tokio::spawn(async move {
                let s = core
                    .wait()
                    .await
                    .map(|s| s.to_string())
                    .unwrap_or_else(|e| format!("wait error: {e}"));
                *exit.lock().await = Some(s);
            });
        }
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let exited = wait_for(
            "core graceful exit",
            Duration::from_secs(20),
            &hint,
            move || {
                let exit = exit.clone();
                Box::pin(async move { exit.lock().await.clone() })
            },
        )
        .await;
        assert!(
            exited.contains("exit status: 0") || exited.contains("exit code: 0"),
            "graceful exit: {exited}"
        );
        assert!(
            sb.state_file.exists(),
            "router state file must be dumped on graceful exit"
        );

        let mut core2 = sb.spawn_core();
        support::wait_reachable(&cli, &sb).await;

        let list_url = format!("{}/api/sessions", sb.webui_url());
        let key_for_list = key.clone();
        let restored = wait_for(
            "session mapping restored after core restart",
            STARTUP,
            &hint,
            move || {
                let cli = cli.clone();
                let url = list_url.clone();
                let key = key_for_list.clone();
                Box::pin(async move {
                    let v = cli
                        .get(&url)
                        .send()
                        .await
                        .ok()?
                        .json::<serde_json::Value>()
                        .await
                        .ok()?;
                    v.to_string().contains(&key).then_some(v)
                })
            },
        )
        .await;
        assert!(
            restored.to_string().contains(&key),
            "session must still be listed after core restart"
        );
        let _ = &mut core2;
    }
    #[cfg(not(unix))]
    {
        // Hard kill: no state dump, so only reachability recovery is
        // assertable on this platform.
        core.kill().await.expect("kill core");
        let _core2 = sb.spawn_core();
        support::wait_reachable(&cli, &sb).await;
    }
}

/// Models-management journey (provider governance): a local stub upstream +
/// provider overlay + model alias → router routes `my-claude` to the stub
/// with the aliased upstream model; admin surface serves stats.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn provider_governance_journey() {
    let sb = Sandbox::new("acceptance", "providers");
    let cli = http_client();

    // Local stub upstream: answers any request with a fixed anthropic-style
    // message, recording the model it was asked for.
    let asked_model: Arc<tokio::sync::Mutex<Option<String>>> = Arc::default();
    let stub = spawn_stub_upstream(asked_model.clone()).await;

    // Provider overlay (read once at router build → write before spawn):
    // provider `stub` pointing at the local upstream + alias my-claude →
    // upstream model `stub-model`.
    std::fs::write(
        sb.path.join("providers.json"),
        format!(
            r#"{{
                "providers": {{
                    "stub": {{ "protocol": "anthropic", "base_url_anthropic": "http://127.0.0.1:{stub}", "api_key": "sk-stub" }}
                }},
                "model_aliases": {{
                    "my-claude": {{ "provider": "stub", "upstream_model": "stub-model" }}
                }}
            }}"#
        ),
    )
    .expect("write providers overlay");

    let _core = sb.spawn_core();
    let router = wait_router_addr(&sb).await;

    let (status, body) = post_json(
        &cli,
        &format!("{router}/v1/messages"),
        serde_json::json!({
            "model": "my-claude",
            "max_tokens": 16,
            "messages": [{ "role": "user", "content": "hi" }]
        }),
    )
    .await
    .expect("router /v1/messages via alias");
    assert_eq!(status, 200, "alias routing: {body}");
    assert_eq!(body["id"].as_str(), Some("msg_stub"), "stub reply: {body}");

    let asked = asked_model.lock().await.clone();
    assert_eq!(
        asked.as_deref(),
        Some("stub-model"),
        "upstream must receive the aliased model id"
    );

    // Admin surface: loopback clients are allowed without a control secret.
    let stats = cli
        .get(format!("{router}/admin/stats"))
        .send()
        .await
        .expect("admin stats");
    assert_eq!(stats.status().as_u16(), 200, "admin stats must serve");
}

/// Native-kernel journey (spike 1.2): `SEBAS_AGENT_PROVIDER_BASE_URL` pointed
/// at a local stub provider; an `agent: "native"` spawn completes a full
/// turn with no real credentials. (The `SEBAS_AGENT_ROUTER_URL` variant is
/// the watchdog's production wiring — `run --router` binds a random port, so
/// it cannot be pre-injected at process level; see COVERAGE.md notes.)
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn native_agent_turn_via_router_journey() {
    let sb = Sandbox::new("acceptance", "native");
    let cli = http_client();

    let asked_model: Arc<tokio::sync::Mutex<Option<String>>> = Arc::default();
    let stub = spawn_stub_upstream(asked_model.clone()).await;

    let base_url = format!("http://127.0.0.1:{stub}");
    let (mut core, dashboard) = sb.spawn_core_inprocess_webui(&[
        ("SEBAS_AGENT_PROVIDER_BASE_URL", base_url.as_str()),
        ("SEBAS_AGENT_PROVIDER_API_KEY", "sk-stub"),
        ("SEBAS_AGENT_MODEL", "stub-model"),
        ("SEBAS_AGENT_MODELS", "stub-model"),
    ]);
    let dashboard_url = format!("http://127.0.0.1:{dashboard}");
    let hint = sb.path.clone();
    let health_cli = cli.clone();
    let health_url = dashboard_url.clone();
    wait_for("in-process webui health", STARTUP, &hint, move || {
        let cli = health_cli.clone();
        let url = health_url.clone();
        Box::pin(async move {
            cli.get(format!("{url}/health"))
                .send()
                .await
                .ok()?
                .text()
                .await
                .ok()
                .map(|b| b.trim() == "ok")
                .filter(|ok| *ok)
        })
    })
    .await;

    let create_url = format!("{dashboard_url}/api/sessions");
    let (status, resp) = post_json(
        &cli,
        &create_url,
        serde_json::json!({ "prompt": "hello", "agent": "native" }),
    )
    .await
    .expect("create native session");
    assert_eq!(status, 201, "native spawn must not be rejected: {resp}");
    let key = resp["key"].as_str().expect("session key").to_string();

    let url = format!("{dashboard_url}/api/sessions/{key}");
    // NOTE: a native turn ends with a "turn summary" element, but the native
    // bridge never sets phase=DONE, so the workbench status stays "Queued"
    // (product finding, see COVERAGE.md). The journey asserts the turn itself
    // completed: summary artifact + the stub was dialed with the configured
    // model.
    let detail = wait_for("native turn to complete", TURN, &hint, move || {
        let cli = cli.clone();
        let url = url.clone();
        Box::pin(async move {
            let v = cli
                .get(&url)
                .send()
                .await
                .ok()?
                .json::<serde_json::Value>()
                .await
                .ok()?;
            let body_text = v["entries"].as_array().map(|b| {
                b.iter()
                    .filter_map(|x| x["content"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })?;
            body_text.contains("turn summary").then_some(v)
        })
    })
    .await;
    assert_eq!(
        detail["current_model"].as_str(),
        Some("stub-model"),
        "native session must carry the configured model: {detail}"
    );
    assert_eq!(
        asked_model.lock().await.clone().as_deref(),
        Some("stub-model"),
        "native agent must have dialed the stub with the configured model"
    );

    let _ = &mut core;
}

/// Project-management journey: register a sandbox dir as a project, list it,
/// then create a session bound to that project dir.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn projects_session_journey() {
    let sb = Sandbox::new("acceptance", "projects");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // Register the sandbox work dir as a project (exists + is a directory).
    let project_dir = sb.path.join("work");
    let (add_status, add_resp) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({ "path": project_dir.to_string_lossy() }),
    )
    .await
    .expect("register project");
    assert!(
        (200..300).contains(&add_status),
        "register project: {add_resp}"
    );

    let projects = cli
        .get(format!("{}/api/projects", sb.webui_url()))
        .send()
        .await
        .expect("list projects")
        .json::<serde_json::Value>()
        .await
        .expect("projects json");
    assert!(
        projects.to_string().contains("work"),
        "registered project must be listed: {projects}"
    );

    // Create a session bound to the project — wire 是稳定 id（不是路径），
    // 注册响应即新条目（带回填的 id）。
    let project_id = add_resp["id"]
        .as_str()
        .expect("registered project entry carries an id")
        .to_string();
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({
            "prompt": "hello",
            "agent": "claude",
            "project_id": project_id
        }),
    )
    .await;
    let transcript = wait_turn_done(&cli, &sb, &key).await;
    assert!(!transcript.is_empty(), "project-bound turn must reply");
}

/// Workbench aggregate journey: agent kinds feed the composer, sessions list
/// and summary rows reflect a newly created session.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn workbench_aggregate_journey() {
    let sb = Sandbox::new("acceptance", "workbench");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // Composer agent dropdown has data (fake-claude registered in config).
    // wire 改名（workbench-agent-wire-fix 3.2）：端点为 /api/agents。
    let kinds = cli
        .get(format!("{}/api/agents", sb.webui_url()))
        .send()
        .await
        .expect("agent kinds")
        .json::<serde_json::Value>()
        .await
        .expect("agent kinds json");
    assert!(
        kinds.to_string().contains("claude"),
        "agent kinds must include the configured claude agent: {kinds}"
    );

    // 0-turn placeholder create → listed.
    let key = create_session(&cli, &sb, serde_json::json!({ "agent": "claude" })).await;
    let rows = cli
        .get(format!("{}/api/sessions", sb.webui_url()))
        .send()
        .await
        .expect("sessions list")
        .json::<serde_json::Value>()
        .await
        .expect("sessions json");
    assert!(
        rows.to_string().contains(&key),
        "new session must appear in the workbench list: {rows}"
    );

    // Summary reflects the session row too.
    let summary = cli
        .get(format!("{}/api/summary", sb.webui_url()))
        .send()
        .await
        .expect("summary")
        .json::<serde_json::Value>()
        .await
        .expect("summary json");
    assert!(
        summary.to_string().contains(&key),
        "summary must reflect the session: {summary}"
    );
}

/// 创建对话框旅程（workbench-interaction-polish 6.3，design D2）：对话框
/// 确认的 wire 面是 0-turn 占位创建（agent 必选 + 可选 mode/model），确认
/// 即激活（set_focus）；取消不落任何东西；占位的首条消息 spawn 子进程。
/// UI 交互本身（对话框开合、预选）由 testsuite-webui 浏览器旅程覆盖——
/// 这条验收面钉的是对话框背后的 wire 契约跨真实进程成立。
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn creation_dialog_journey() {
    let sb = Sandbox::new("acceptance", "creation-dialog");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // 1) 对话框 agent 数据源就绪（agent 必选的词汇表）。
    let kinds = cli
        .get(format!("{}/api/agents", sb.webui_url()))
        .send()
        .await
        .expect("agent kinds")
        .json::<serde_json::Value>()
        .await
        .expect("agent kinds json");
    assert!(
        kinds.to_string().contains("claude"),
        "dialog agent source must include claude: {kinds}"
    );

    // 2) 确认（mode=allow，模型缺省）→ 0-turn 占位创建且激活。
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({ "agent": "claude", "mode": "allow" }),
    )
    .await;
    let detail = cli
        .get(format!("{}/api/sessions/{key}", sb.webui_url()))
        .send()
        .await
        .expect("placeholder detail")
        .json::<serde_json::Value>()
        .await
        .expect("detail json");
    assert_eq!(
        detail["desired_mode"].as_str(),
        Some("allow"),
        "dialog mode choice must land on the wire: {detail}"
    );
    // 激活：占位即成为焦点会话（composer 就地进入跟随态的数据源）。
    let summary = cli
        .get(format!("{}/api/summary", sb.webui_url()))
        .send()
        .await
        .expect("summary")
        .json::<serde_json::Value>()
        .await
        .expect("summary json");
    assert_eq!(
        summary["active_session_key"].as_str(),
        Some(key.as_str()),
        "confirmed placeholder must be the focused session: {summary}"
    );

    // 3) 首条消息 spawn 子进程，turn 收敛 Done。
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("first message");
    assert_eq!(status, 200, "first message: {resp}");
    let transcript = wait_turn_done(&cli, &sb, &key).await;
    assert!(
        transcript.contains("hello") && transcript.contains("world"),
        "first message must spawn and answer: {transcript:?}"
    );

    // 4) agent 词汇门（对话框的 agent 词汇表 = 配置键名/native；旧 backend
    //    词汇在 wire 上被类型化拒绝——对话框选不出也发不出这些值）。
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "agent": "acp:claude" }),
    )
    .await
    .expect("legacy agent create");
    assert!(
        status >= 400 && status < 500,
        "legacy backend word must be rejected: {status} {resp}"
    );
}

/// Router downstream-auth journey: with `auth_token` configured and the
/// router NOT in debug mode (debug skips downstream auth), the proxy surface
/// rejects tokenless requests. The authorized-path 200 is covered by every
/// other journey riding the debug `test` provider.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn router_downstream_auth_journey() {
    let sb = Sandbox::new("acceptance", "auth");
    sb.set_router_auth_token("sk-gw-test-token");
    let cli = http_client();
    let _core = sb.spawn_core_router_auth();
    let router = wait_router_addr(&sb).await;

    let url = format!("{router}/v1/messages");
    let payload = serde_json::json!({
        "model": "claude-x",
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "hi" }]
    });

    let unauth = cli
        .post(&url)
        .json(&payload)
        .send()
        .await
        .expect("tokenless request");
    assert_eq!(
        unauth.status().as_u16(),
        401,
        "tokenless proxy request must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// add-remote-execution-node 9.4：webui + 真 sebas-node 的远端节点旅程
// ═══════════════════════════════════════════════════════════════════════════

/// 从 core 日志里等出一次性 bootstrap 配对 token（只打印一次）。
async fn wait_bootstrap_token(sb: &Sandbox) -> String {
    let log = sb.core_log.clone();
    let hint = sb.path.clone();
    support::wait_for(
        "core 打印一次性 bootstrap 配对 token",
        Duration::from_secs(20),
        &hint,
        move || {
            let log = log.clone();
            Box::pin(async move {
                let text = std::fs::read_to_string(&log).ok()?;
                text.split("有效）：")
                    .nth(1)
                    .and_then(|rest| {
                        let token: String = rest
                            .trim_start()
                            .chars()
                            .take_while(|c| c.is_ascii_hexdigit())
                            .collect();
                        (token.len() == 64).then_some(token)
                    })
            })
        },
    )
    .await
}

/// 等某节点在 `GET /api/nodes` 上进入给定状态（经 HTTP，不查内部结构）。
async fn wait_node_status(cli: &reqwest::Client, sb: &Sandbox, node_id: &str, want: &str) {
    let url = format!("{}/api/nodes", sb.webui_url());
    let want_s = want.to_string();
    let node_s = node_id.to_string();
    let hint = sb.path.clone();
    let got = support::wait_for(
        &format!("节点 {node_id} 状态变为 {want}"),
        Duration::from_secs(30),
        &hint,
        move || {
            let cli = cli.clone();
            let url = url.clone();
            let want = want_s.clone();
            let node = node_s.clone();
            Box::pin(async move {
                let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                let nodes = v.get("nodes")?.as_array()?.clone();
                let found = nodes
                    .iter()
                    .find(|n| n.get("id").and_then(|i| i.as_str()) == Some(node.as_str()))?;
                let status = found.get("status").and_then(|s| s.as_str())?;
                (status == want).then_some(status.to_string())
            })
        },
    )
    .await;
    assert_eq!(got, want);
}

/// 等 `/api/sessions` 里出现满足谓词**且**额外条件成立的一行；返回该行。
async fn wait_session_row(
    cli: &reqwest::Client,
    sb: &Sandbox,
    what: &str,
    pred: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
) -> serde_json::Value {
    let cli2 = cli.clone();
    let url = format!("{}/api/sessions", sb.webui_url());
    let hint = sb.path.clone();
    // 谓词按 Arc 共享：每次轮询的 future 必须是 `'static`，借用外层的 Fn 不行。
    let pred = std::sync::Arc::new(pred);
    support::wait_for(what, Duration::from_secs(30), &hint, move || {
        let cli = cli2.clone();
        let url = url.clone();
        let pred = pred.clone();
        Box::pin(async move {
            let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
            v.get("recent_sessions")?
                .as_array()?
                .iter()
                .find(|r| pred(r))
                .cloned()
        })
    })
    .await
}

/// 9.4 验收旅程（webui + 真 `sebas-node`，两个进程）。
///
/// 覆盖：项目带节点注册且路径由**节点**判定 → 会话归属带节点 → 节点离线呈现
/// 并在提交前如实拒绝 → 节点回归免刷新恢复 → 悬空审批呈现为**等待而不是运行**。
///
/// 诚实边界（在断言处逐条注明，不假装覆盖）：
/// - **mode 差异**：`POST /api/sessions` / core channel `Spawn` 没有 mode 字段，
///   且 core 只在节点重连对账时读到 `SessionSummary.desired_mode`——所以本旅程
///   无法制造 desired≠effective；该呈现由前端单测（dashboard.test.ts 的
///   `mode-mismatch`）与 `tests/node_link_e2e_test.rs`（节点侧）覆盖。
/// - **审批请求可达性**：实测 webui `/ws`（review-card 投递面）在整段旅程里
///   一帧未投（连 `session.updated` 都没有），因此"悬空请求可达"只断言到
///   HTTP 层的 `parked_approvals` 计数与 `waiting` 呈现；`/ws` 投递面待查
///   （属 src/ 的通道/WS 接线，不是 sebas-webui）。
/// - **审批决议**：核心尚未把 `ApprovalAnswer` 路由到节点（实测返回 404），
///   故不断言决议生效。
/// - 真实 agent CLI（远端 turn）在沙箱不可用：节点只有 echo 执行体。
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke testsuite-acceptance"]
async fn remote_node_workbench_journey() {
    let sb = Sandbox::new("acceptance", "remote-node");
    sb.enable_node_link();
    let node_work = sb.node_work_dir();
    let cli = http_client();
    let mut core = sb.spawn_core();
    let mut webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // 1) 配对：bootstrap token 只打印一次，节点用它换长期凭据。
    let token = wait_bootstrap_token(&sb).await;
    let mut node = sb.spawn_node("itest-node", Some(&token));
    wait_node_status(&cli, &sb, "itest-node", "online").await;

    // 本机节点恒在列：隐式节点也有状态，不能因为"看不见"就说"没有"。
    let nodes = cli
        .get(format!("{}/api/nodes", sb.webui_url()))
        .send()
        .await
        .expect("nodes")
        .json::<serde_json::Value>()
        .await
        .expect("nodes json");
    let ids: Vec<&str> = nodes["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(ids.contains(&"local"), "本机节点必须在列: {nodes}");
    assert!(ids.contains(&"itest-node"), "配对节点必须在列: {nodes}");
    assert_eq!(nodes["remote_available"], true);

    // 2) 远端项目注册：主控不 stat 这个路径，**节点**说了算。
    let (status, add) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({
            "path": node_work.to_string_lossy(),
            "node_id": "itest-node",
        }),
    )
    .await
    .expect("register remote project");
    assert_eq!(status, 201, "远端项目注册应由节点校验通过: {add}");
    assert_eq!(add["node_id"], "itest-node", "注册条目带节点维度: {add}");
    let project_id = add["id"].as_str().expect("project id").to_string();

    // 同一路径换一个节点是**另一个**项目（id 不同）——本机同路径不算同一个。
    assert_ne!(
        project_id,
        sebas_webui::projects::project_id_for(&node_work.to_string_lossy()),
        "远端项目的 id 必须与本机同路径项目不同"
    );

    // 3) 远端会话：session 归属带节点 + 项目 id 按 `(节点, 路径)` 派生。
    //
    // GAP（core，已实测）：core 的 `spawn_on` 收了 prompt 却只存进 meta 预览，
    // 不向节点发 `SessionOp::Prompt`——所以创建时给的 prompt 不会产生 turn。
    // 这里先建会话（prompt 仅为满足非空），随后用 message 端点投递受门控输入。
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({ "prompt": "warmup", "agent": "echo", "project_id": project_id }),
    )
    .await;
    let expect_pid = sebas_webui::projects::project_id_for_on("itest-node", &node_work.to_string_lossy());
    let key_for_row = key.clone();
    let row = wait_session_row(&cli, &sb, "远端会话行带节点与项目", move |r| {
        r["encoded_key"].as_str() == Some(key_for_row.as_str())
    })
    .await;
    assert_eq!(
        row["remote"]["node_id"], "itest-node",
        "会话必须标注所属节点: {row}"
    );
    assert_eq!(
        row["remote"]["node_status"], "online",
        "节点在线时如实呈现 online: {row}"
    );
    assert_eq!(
        row["project_id"], expect_pid,
        "会话必须挂到 `(节点, 路径)` 那条项目上: {row}"
    );

    // 4) 悬空审批：受门控动作经 message 投递 → 节点停驻 → 会话呈现为**等待**。
    let (msg_status, _) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "run: ls -la" }),
    )
    .await
    .expect("gated message");
    assert!((200..300).contains(&msg_status), "受门控输入应被接受");

    let key_for_wait = key.clone();
    let waiting = wait_session_row(&cli, &sb, "会话因悬空审批转为等待", move |r| {
        r["encoded_key"].as_str() == Some(key_for_wait.as_str())
            && r["remote"]["parked_approvals"].as_u64().unwrap_or(0) >= 1
    })
    .await;
    assert_eq!(
        waiting["status_slug"], "waiting",
        "在等人批的会话不得读作运行中: {waiting}"
    );
    assert_ne!(waiting["status_slug"], "working");

    // 审批请求**可达**（HTTP 面）：悬空计数与成因都在行上。review-card 的
    // /ws 投递面在沙箱里一帧未投（见文件头 GAP），因此这一条只覆盖 HTTP 面。
    assert!(
        waiting["remote"]["parked_approvals"].as_u64().unwrap_or(0) >= 1,
        "悬空审批数必须可见: {waiting}"
    );

    // 5) 节点离线：如实呈现 + 提交**前**就被拒（不排队、不建占位）。
    node.kill().await.expect("kill node");
    let _ = node.wait().await;
    wait_node_status(&cli, &sb, "itest-node", "offline").await;

    let key_for_offline = key.clone();
    let offline_row = wait_session_row(&cli, &sb, "远端会话行标出节点离线与成因", move |r| {
        r["encoded_key"].as_str() == Some(key_for_offline.as_str())
            && r["remote"]["node_status"].as_str() == Some("offline")
    })
    .await;
    assert!(
        offline_row["remote"]["node_cause"].as_str().is_some_and(|c| !c.is_empty()),
        "离线必须给出成因，不能只写『不可用』: {offline_row}"
    );

    // 远端项目的"可用"等于节点在线（主控不做本地 stat）。
    let branch = cli
        .get(format!("{}/api/projects/{project_id}/branch", sb.webui_url()))
        .send()
        .await
        .expect("branch")
        .json::<serde_json::Value>()
        .await
        .expect("branch json");
    assert_eq!(branch["accessible"], false, "节点离线 ⇒ 项目不可用: {branch}");
    assert_eq!(branch["node_id"], "itest-node");

    // composer 门禁的服务端对应行为：提交被如实拒绝，且点名节点与原因。
    let (create_status, create_body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "x", "agent": "echo", "project_id": project_id }),
    )
    .await
    .expect("offline create");
    assert_ne!(create_status, 201, "节点离线时不得建立会话: {create_body}");
    let cause = create_body["error"].as_str().unwrap_or_default();
    assert!(
        cause.contains("itest-node"),
        "拒绝必须点名节点: {create_body}"
    );

    // 6) 节点带原凭据重启：webui/core 都没动 → 免刷新恢复在线。
    let mut node = sb.spawn_node("itest-node", None);
    wait_node_status(&cli, &sb, "itest-node", "online").await;

    // 收尾：显式停掉子进程（Drop 也会收，但这里让端口与 socket 干净释放）。
    node.kill().await.ok();
    core.kill().await.ok();
    webui.kill().await.ok();
}


/// （add-agent-mode-selection）远端节点 mode 旅程（真 sebas-node + EchoBody，
/// 零真模型调用）：
/// - 创建带 mode=allow → 投影 desired_mode=allow，`run:` 门控动作**不停驻**；
/// - 中途切回 ask（节点存活时）→ 后续 `run:` 重新进入 waiting；
/// - 节点重连对账补回执行事实（与 remote_node_workbench_journey 同款机制：
///   沙箱里节点事件不实时进主控转写，重连后可见）；
/// - 节点离线时带 mode 创建照旧在提交前被拒（点名节点）。
///
/// 诚实边界：EchoBody 不声明 `enforces_mode`，节点如实回报 effective=None
/// （desired≠effective 正是 execution-node spec 要求的"强制不了要说出来"）；
/// 门控放行/停驻由节点按 desired mode 判定，`allow` 放行会留审计。
#[tokio::test]
#[ignore = "process-level acceptance; run with -- --ignored or invoke testsuite-acceptance"]
async fn remote_node_mode_journey() {
    let sb = Sandbox::new("acceptance", "remote-node-mode");
    sb.enable_node_link();
    let node_work = sb.node_work_dir();
    let cli = http_client();
    let mut core = sb.spawn_core();
    let mut webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // 配对上线 + 远端项目注册（与 remote_node_workbench_journey 同款装配）。
    let token = wait_bootstrap_token(&sb).await;
    let mut node = sb.spawn_node("itest-node", Some(&token));
    wait_node_status(&cli, &sb, "itest-node", "online").await;
    let (status, add) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({
            "path": node_work.to_string_lossy(),
            "node_id": "itest-node",
        }),
    )
    .await
    .expect("register remote project");
    assert_eq!(status, 201, "远端项目注册: {add}");
    let project_id = add["id"].as_str().expect("project id").to_string();

    // 1) 创建带 mode=allow：mode 随放置链路（Spawn 帧 → 节点 spawn op）送达；
    //    投影 desired_mode 如实呈现。
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({
            "prompt": "warmup",
            "agent": "echo",
            "project_id": project_id,
            "mode": "allow",
        }),
    )
    .await;
    let key_for_row = key.clone();
    let row = wait_session_row(&cli, &sb, "远端会话行带 desired mode", move |r| {
        r["encoded_key"].as_str() == Some(key_for_row.as_str())
            && r["remote"]["desired_mode"].as_str() == Some("allow")
    })
    .await;
    assert_eq!(row["remote"]["node_id"], "itest-node", "会话归属节点: {row}");

    // 2) allow 下受门控动作直接执行：`run:` 投递成功且**不产生**悬空审批
    //    （ask 下同输入会停在 waiting——见 remote_node_workbench_journey）。
    //    执行事实（echo 应答）经节点重连对账补回后可见。
    let (msg_status, _) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "run: ls -la" }),
    )
    .await
    .expect("gated message under allow");
    assert!((200..300).contains(&msg_status));
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let v = cli
            .get(format!("{}/api/sessions", sb.webui_url()))
            .send()
            .await
            .expect("sessions")
            .json::<serde_json::Value>()
            .await
            .expect("sessions json");
        let r = v["recent_sessions"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|r| r["encoded_key"].as_str() == Some(key.as_str()))
            .expect("row");
        assert_eq!(
            r["remote"]["parked_approvals"].as_u64().unwrap_or(0),
            0,
            "allow 模式不得停驻审批: {r}"
        );
        assert_ne!(r["status_slug"], "waiting", "allow 下不该等人批: {r}");
    }

    // 3) 中途切回 ask：SetMode 经链路送达；后续 `run:` 重新被门控为 waiting
    //    （悬空审批是控制面主动对账的面，这一步**实时**可见）。
    let (switch_status, switch_body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/mode", sb.webui_url()),
        serde_json::json!({ "mode": "ask" }),
    )
    .await
    .expect("switch mode mid-session");
    assert_eq!(switch_status, 200, "mode 切换应送达: {switch_body}");
    let key_for_ask = key.clone();
    wait_session_row(&cli, &sb, "投影 desired 更新为 ask", move |r| {
        r["encoded_key"].as_str() == Some(key_for_ask.as_str())
            && r["remote"]["desired_mode"].as_str() == Some("ask")
    })
    .await;
    let _ = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "run: echo again" }),
    )
    .await
    .expect("gated message under ask");
    let key_for_wait = key.clone();
    let waiting = wait_session_row(&cli, &sb, "ask 下门控动作恢复 waiting", move |r| {
        r["encoded_key"].as_str() == Some(key_for_wait.as_str())
            && r["remote"]["parked_approvals"].as_u64().unwrap_or(0) >= 1
    })
    .await;
    assert_eq!(
        waiting["status_slug"], "waiting",
        "切回 ask 后必须重新等人批: {waiting}"
    );

    // 4) 节点重连对账：echo 对 `run:` 的应答经对账补回转写。
    node.kill().await.expect("kill node");
    let _ = node.wait().await;
    let mut node = sb.spawn_node("itest-node", None);
    wait_node_status(&cli, &sb, "itest-node", "online").await;
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let transcript = wait_for(
        "对账补回 allow 会话的 echo 应答",
        TURN,
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = detail_url.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                Box::pin(async move {
                    let v = cli
                        .get(&url)
                        .send()
                        .await
                        .ok()?
                        .json::<serde_json::Value>()
                        .await
                        .ok()?;
                    let text = v
                        .get("entries")?
                        .as_array()?
                        .iter()
                        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
                        .collect::<Vec<_>>()
                        .join("");
                    text.contains("run: ls -la").then_some(text)
                })
            }
        },
    )
    .await;
    assert!(
        transcript.contains("run: ls -la"),
        "allow 会话的门控动作应执行并有应答: {transcript:?}"
    );

    // 5) 节点离线：带 mode 的创建照旧在提交前被拒，点名节点。
    node.kill().await.expect("kill node");
    let _ = node.wait().await;
    wait_node_status(&cli, &sb, "itest-node", "offline").await;
    let (create_status, create_body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "prompt": "x",
            "agent": "echo",
            "project_id": project_id,
            "mode": "allow",
        }),
    )
    .await
    .expect("offline create with mode");
    assert_ne!(create_status, 201, "节点离线不得建会话: {create_body}");
    let cause = create_body["error"].as_str().unwrap_or_default();
    assert!(cause.contains("itest-node"), "拒绝点名节点: {create_body}");

    node.kill().await.ok();
    core.kill().await.ok();
    webui.kill().await.ok();
}
