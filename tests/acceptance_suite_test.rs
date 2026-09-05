//! Acceptance suite (acceptance-suite): multi-step, journey-level cases that
//! cross several capabilities over real process boundaries. Sandbox rules are
//! the same as `core_flow_e2e_test` (support::Sandbox): everything inside a
//! throwaway dir, probed ports, no operator instance touched.
//!
//! Opt-in only: `cargo test --test acceptance_suite_test -- --ignored`
//! or `invoke accept` (`--case <name>` filters). Coverage accounting for
//! these cases lives in `tests/acceptance/COVERAGE.md`.

use std::sync::Arc;
use std::time::Duration;

mod support;

use support::{http_client, post_json, wait_for, wait_router_addr, Sandbox};

const TURN: Duration = Duration::from_secs(30);
const STARTUP: Duration = Duration::from_secs(30);

async fn create_session(
    cli: &reqwest::Client,
    sb: &Sandbox,
    body: serde_json::Value,
) -> String {
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
    detail["body"]
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
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
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
        serde_json::json!({ "prompt": "hello", "backend": "acp" }),
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
        let exited = wait_for("core graceful exit", Duration::from_secs(20), &hint, move || {
            let exit = exit.clone();
            Box::pin(async move { exit.lock().await.clone() })
        })
        .await;
        assert!(exited.contains("code: 0"), "graceful exit: {exited}");
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
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
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
/// at a local stub provider; a `backend: "native"` spawn completes a full
/// turn with no real credentials. (The `SEBAS_AGENT_ROUTER_URL` variant is
/// the watchdog's production wiring — `run --router` binds a random port, so
/// it cannot be pre-injected at process level; see COVERAGE.md notes.)
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
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
        serde_json::json!({ "prompt": "hello", "backend": "native" }),
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
            let body_text = v["body"].as_array().map(|b| {
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
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
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

    // Create a session bound to the project dir.
    let key = create_session(
        &cli,
        &sb,
        serde_json::json!({
            "prompt": "hello",
            "backend": "acp",
            "project_dir": project_dir.to_string_lossy()
        }),
    )
    .await;
    let transcript = wait_turn_done(&cli, &sb, &key).await;
    assert!(!transcript.is_empty(), "project-bound turn must reply");
}

/// Workbench aggregate journey: agent kinds feed the composer, sessions list
/// and summary rows reflect a newly created session.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
async fn workbench_aggregate_journey() {
    let sb = Sandbox::new("acceptance", "workbench");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    support::wait_reachable(&cli, &sb).await;

    // Composer agent dropdown has data (fake-claude registered in config).
    let kinds = cli
        .get(format!("{}/api/agent-kinds", sb.webui_url()))
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
    let key = create_session(&cli, &sb, serde_json::json!({ "backend": "acp" })).await;
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

/// Router downstream-auth journey: with `auth_token` configured and the
/// router NOT in debug mode (debug skips downstream auth), the proxy surface
/// rejects tokenless requests. The authorized-path 200 is covered by every
/// other journey riding the debug `test` provider.
#[tokio::test]
#[ignore = "acceptance journey; run with -- --ignored or invoke accept"]
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

/// Minimal local HTTP upstream for the provider-governance journey: answers
/// every request with a fixed anthropic-style message JSON and records the
/// `model` field of the last request body.
async fn spawn_stub_upstream(
    asked_model: Arc<tokio::sync::Mutex<Option<String>>>,
) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub upstream");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let asked = asked_model.clone();
            tokio::spawn(async move {
                // Read until end of headers, then exactly content-length bytes.
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    if buf.len() >= chunk.len() * 4 {
                        return; // runaway request; drop
                    }
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                        // ensure full body arrived too
                        let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            break pos + 4 + len;
                        }
                    }
                };
                let text = String::from_utf8_lossy(&buf[..header_end]);
                if let Some(model_key) = find_json_string(&text, "\"model\":\"") {
                    *asked.lock().await = Some(model_key);
                }
                let body = r#"{"id":"msg_stub","type":"message","role":"assistant","model":"stub-model","content":[{"type":"text","text":"stub reply"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Crude extractor for the first `"model":"…"` value in a JSON body —
/// enough for the stub's recording purposes.
fn find_json_string(text: &str, key_prefix: &str) -> Option<String> {
    let start = text.find(key_prefix)? + key_prefix.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}
