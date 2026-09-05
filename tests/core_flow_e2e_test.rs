//! Process-level e2e for the core flows in the detached (watchdog) topology:
//! a real core child (`sebas run --gateway --debug`) plus a standalone
//! `sebas webui` process connected through the core session channel.
//!
//! Every case runs in a throwaway sandbox (`support::Sandbox`) — config file,
//! state DB, provider overlay and channel socket all inside it; the webui
//! binds a probed free port. Nothing touches the operator's real `~/.sebas`.
//!
//! Opt-in only (`#[ignore]`): process spawning is seconds-scale, so these
//! never run in the default `cargo test` gate. Run them with
//! `cargo test --test core_flow_e2e_test -- --ignored` or `invoke e2e`.
//! Any panic keeps the sandbox dir (with core.log / webui.log) for
//! postmortem — the path is printed on drop.

use std::sync::Arc;
use std::time::Duration;

mod support;

use support::{
    http_client, post_json, wait_for, wait_gateway_addr, wait_reachable,
    wait_unreachable_with_cause, webui_healthy, Sandbox,
};

/// Startup: core + standalone webui come up, webui reports the core channel
/// reachable and /health serves.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn detached_startup_reports_reachability() {
    let sb = Sandbox::new("core_flow_e2e", "startup");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);

    wait_reachable(&cli, &sb).await;
    let health_url = format!("{}/health", sb.webui_url());
    let hint = sb.path.clone();
    let healthy = wait_for("webui /health ok", Duration::from_secs(10), &hint, move || {
        let cli = cli.clone();
        let url = health_url.clone();
        Box::pin(async move {
            cli.get(&url)
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
    assert!(healthy, "webui /health must report ok once serving");
}

/// Full session round-trip over the webui HTTP surface: create (ACP) →
/// core channel → fake-claude turn → Done with the stub's reply visible.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn session_round_trip_via_webui_http() {
    let sb = Sandbox::new("core_flow_e2e", "round-trip");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "hello", "backend": "acp" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"]
        .as_str()
        .expect("key in create response")
        .to_string();
    assert!(!key.is_empty());

    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();
    let detail = wait_for(
        "session turn to reach Done",
        Duration::from_secs(25),
        &hint,
        move || {
            let cli = cli.clone();
            let url = detail_url.clone();
            Box::pin(async move {
                let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                let done = v["status_slug"].as_str() == Some("done")
                    || v["status"]
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case("done"));
                done.then_some(v)
            })
        },
    )
    .await;

    let transcript = detail["body"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["content"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    assert!(
        transcript.contains("hello") && transcript.contains("world"),
        "turn transcript must carry fake-claude's reply, got: {transcript:?}"
    );
}

/// The built-in debug gateway answers `model = "test"` over /v1/messages.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn gateway_debug_provider_serves_messages() {
    let sb = Sandbox::new("core_flow_e2e", "gateway");
    let cli = http_client();
    let _core = sb.spawn_core();

    let gateway = wait_gateway_addr(&sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{gateway}/v1/messages"),
        serde_json::json!({
            "model": "test",
            "max_tokens": 16,
            "messages": [{ "role": "user", "content": "hi" }]
        }),
    )
    .await
    .expect("gateway /v1/messages");
    assert_eq!(status, 200, "debug gateway: {body}");
    assert_eq!(
        body["id"].as_str(),
        Some("msg_test_debug"),
        "debug provider fixed reply id: {body}"
    );
}

/// A webui presenting a wrong SEBAS_CORE_SECRET must never fake a connected
/// state: /health still serves, reachability stays false with a cause.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn wrong_core_secret_refuses_connection() {
    let sb = Sandbox::new("core_flow_e2e", "wrong-secret");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui("definitely-not-the-secret");

    let cause = wait_unreachable_with_cause(&cli, &sb).await;
    assert!(!cause.is_empty(), "unreachable cause must be reported");

    assert_eq!(
        webui_healthy(&cli, &sb).await,
        Some(true),
        "webui must keep serving while unreachable"
    );
}

/// Core lifecycle is honestly visible on the webui side: kill the core →
/// reachability flips false with a cause; restart it → flips back to true.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn reachability_flips_across_core_restart() {
    let sb = Sandbox::new("core_flow_e2e", "restart");
    let cli = http_client();
    let mut core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    core.kill().await.expect("kill core");
    wait_unreachable_with_cause(&cli, &sb).await;
    assert_eq!(
        webui_healthy(&cli, &sb).await,
        Some(true),
        "webui must keep serving while the core is down"
    );

    let _core2 = sb.spawn_core();
    wait_reachable(&cli, &sb).await;
}

/// Graceful exit (SIGTERM, unix-gated like sigterm_cleanup_test): the core
/// removes the channel socket and dumps session state.
#[cfg(unix)]
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke e2e"]
async fn graceful_exit_removes_channel_socket() {
    let sb = Sandbox::new("core_flow_e2e", "sigterm");
    // SEBAS_TEST_SPAWN_SESSION=1 mints one fake-claude session at startup so
    // the state dump has content — same affordance sigterm_cleanup_test uses.
    let mut core = sb.spawn_core_extra(&[("SEBAS_TEST_SPAWN_SESSION", "1")]);
    let pid = core.id().expect("core pid") as libc::pid_t;

    let socket = sb.channel_path.clone();
    let hint = sb.path.clone();
    wait_for("channel socket to appear", Duration::from_secs(15), &hint, move || {
        let socket = socket.clone();
        Box::pin(async move { socket.exists().then_some(()) })
    })
    .await;
    // Give the affordance session time to register before the signal,
    // mirroring sigterm_cleanup_test's child-registration budget.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Reap the child in a side task; record how it exited.
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

    unsafe { libc::kill(pid, libc::SIGTERM) };

    let exited = wait_for(
        "core to exit after SIGTERM",
        Duration::from_secs(20),
        &hint,
        move || {
            let exit = exit.clone();
            Box::pin(async move { exit.lock().await.clone() })
        },
    )
    .await;
    assert!(
        exited.contains("code: 0"),
        "graceful exit must succeed, got: {exited}"
    );
    assert!(
        !sb.channel_path.exists(),
        "channel socket must be removed on graceful exit"
    );
    assert!(
        sb.state_file.exists(),
        "session state must be dumped on graceful exit"
    );
}
