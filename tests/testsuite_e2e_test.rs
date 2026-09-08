//! Process-level e2e for the core flows in the detached (watchdog) topology:
//! a real core child (`sebas run --router --debug`) plus a standalone
//! `sebas webui` process connected through the core session channel.
//!
//! Every case runs in a throwaway sandbox (`support::Sandbox`) — config file,
//! state DB, provider overlay and channel socket all inside it; the webui
//! binds a probed free port. Nothing touches the operator's real `~/.sebas`.
//!
//! Opt-in only (`#[ignore]`): process spawning is seconds-scale, so these
//! never run in the default `cargo test` gate. Run them with
//! `cargo test --test testsuite_e2e_test -- --ignored` or `invoke testsuite-e2e`.
//! Any panic keeps the sandbox dir (with core.log / webui.log) for
//! postmortem — the path is printed on drop.

#[cfg(unix)]
use std::sync::Arc;
use std::time::Duration;

mod support;

use support::{
    http_client, post_json, wait_for, wait_router_addr, wait_reachable, wait_secret_file,
    wait_unreachable_with_cause, webui_healthy, Sandbox,
};
#[cfg(target_os = "linux")]
use support::wait_supervised_core_pid;

/// Startup: core + standalone webui come up, webui reports the core channel
/// reachable and /health serves.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn detached_startup_reports_reachability() {
    let sb = Sandbox::new("testsuite_e2e", "startup");
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
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn session_round_trip_via_webui_http() {
    let sb = Sandbox::new("testsuite_e2e", "round-trip");
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

/// The built-in debug router answers `model = "test"` over /v1/messages.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn router_debug_provider_serves_messages() {
    let sb = Sandbox::new("testsuite_e2e", "router");
    let cli = http_client();
    let _core = sb.spawn_core();

    let router = wait_router_addr(&sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{router}/v1/messages"),
        serde_json::json!({
            "model": "test",
            "max_tokens": 16,
            "messages": [{ "role": "user", "content": "hi" }]
        }),
    )
    .await
    .expect("router /v1/messages");
    assert_eq!(status, 200, "debug router: {body}");
    assert_eq!(
        body["id"].as_str(),
        Some("msg_test_debug"),
        "debug provider fixed reply id: {body}"
    );
}

/// A webui presenting a wrong SEBAS_CORE_SECRET must never fake a connected
/// state: /health still serves, reachability stays false with a cause.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn wrong_core_secret_refuses_connection() {
    let sb = Sandbox::new("testsuite_e2e", "wrong-secret");
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
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn reachability_flips_across_core_restart() {
    let sb = Sandbox::new("testsuite_e2e", "restart");
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
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn graceful_exit_removes_channel_socket() {
    let sb = Sandbox::new("testsuite_e2e", "sigterm");
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
    // ExitStatus Display is platform-dependent ("exit status: 0" on unix,
    // "exit code: 0" on Windows) — accept either.
    assert!(
        exited.contains("exit status: 0") || exited.contains("exit code: 0"),
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

/// fail-fast-on-startup-errors（tasks 2.3/4.2）：`sebas core` 对 garbage
/// 配置在 ready 之前 fatal —— 进程以 EX_TEMPFAIL (75) 退出，stderr 最后一
/// 行是 `startup-failure: <原因>` 摘要，且 `SEBAS_STARTUP_ERROR_FILE`（若
/// 设置）被覆盖写入同一行。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn startup_failure_core_exits_75_with_summary() {
    let sb = Sandbox::new("testsuite_e2e", "startup-failure-core");
    // 覆写为 garbage 配置：TOML 解析必败 → ready 前 fatal。
    std::fs::write(&sb.config_path, "not [ valid toml").expect("write garbage config");
    let error_file = sb.path.join("startup-error.log");
    let ef = support::forward_slash(&error_file);
    let cfg = support::forward_slash(&sb.config_path);

    let mut child = sb.spawn(
        &["core", "-c", &cfg],
        &sb.core_secret,
        &[("SEBAS_STARTUP_ERROR_FILE", &ef)],
        &sb.core_log,
    );
    let status = child.wait().await.expect("wait sebas core");
    assert_eq!(
        status.code(),
        Some(sebas::startup_failure::EXIT_STARTUP_FAILURE),
        "startup failure must exit 75 (EX_TEMPFAIL), got {status}"
    );

    // stderr 最后一行 = 摘要（单行，前缀固定）。
    let log = std::fs::read_to_string(&sb.core_log).expect("read core log (stderr)");
    let last = log
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .expect("stderr not empty");
    assert!(
        last.starts_with("startup-failure: "),
        "last stderr line must be the startup-failure summary, got: {last:?}"
    );

    // 错误摘要文件与 stderr 末行携带同一摘要。
    let file = std::fs::read_to_string(&error_file)
        .expect("SEBAS_STARTUP_ERROR_FILE must be written (overwrite)");
    assert_eq!(
        file.trim(),
        last.trim(),
        "error file must carry the same single-line summary as the stderr tail"
    );
}

/// fail-fast-on-startup-errors（tasks 2.3/4.2）：watchdog 形态（`sebas run`）
/// 对 garbage 配置同样在自身启动阶段 fatal → 75 + 摘要（config 解析发生在
/// watchdog 进程内、任何子进程 spawn 之前）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn startup_failure_run_exits_75_with_summary() {
    let sb = Sandbox::new("testsuite_e2e", "startup-failure-run");
    std::fs::write(&sb.config_path, "not [ valid toml").expect("write garbage config");
    let error_file = sb.path.join("startup-error.log");
    let ef = support::forward_slash(&error_file);
    let cfg = support::forward_slash(&sb.config_path);

    let mut child = sb.spawn(
        &["run", "-c", &cfg],
        &sb.core_secret,
        &[("SEBAS_STARTUP_ERROR_FILE", &ef)],
        &sb.core_log,
    );
    let status = child.wait().await.expect("wait sebas run");
    assert_eq!(
        status.code(),
        Some(sebas::startup_failure::EXIT_STARTUP_FAILURE),
        "watchdog startup failure must exit 75, got {status}"
    );

    let log = std::fs::read_to_string(&sb.core_log).expect("read run log (stderr)");
    let last = log
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .expect("stderr not empty");
    assert!(
        last.starts_with("startup-failure: "),
        "last stderr line must be the startup-failure summary, got: {last:?}"
    );
    let file = std::fs::read_to_string(&error_file).expect("startup error file written");
    assert_eq!(file.trim(), last.trim());
}

/// harden-core-channel-deployment §5.1 — no-secret assembly journey (the
/// live "socket absent" incident as a regression case): core and standalone
/// webui start with NO `SEBAS_CORE_SECRET` in env. The core auto-arms with a
/// minted secret published to the secret file; the webui discovers it from
/// the same `-c` config. Reachability must come up and a full ACP session
/// round-trip must complete. The pre-existing env-injection cases above
/// cover the "existing path does not regress" half of the spec.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn channel_no_secret_assembly_serves_session_round_trip() {
    let sb = Sandbox::new("testsuite_e2e", "no-secret");
    let cli = http_client();
    let _core = sb.spawn_core_no_secret();
    let _webui = sb.spawn_webui_no_secret();

    // Auto-arm must have published a minted secret where D1 says it lives
    // (wait covers both existence and content; the path is resolved by the
    // same function the core and webui use).
    let published = wait_secret_file(&sb).await;
    assert!(!published.is_empty());

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

    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();
    let detail = wait_for(
        "no-secret session turn to reach Done",
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

/// harden-core-channel-deployment §5.2 — secret rotation self-heal: on a
/// no-secret assembly each core boot mints a FRESH secret (rotation). Kill
/// the core and restart it with the same config; the running webui (file
/// discovery, re-read per connect) must recover WITHOUT a restart, with an
/// honest cause while down.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn channel_secret_rotation_heals_running_webui() {
    let sb = Sandbox::new("testsuite_e2e", "rotation");
    let cli = http_client();
    let mut core = sb.spawn_core_no_secret();
    let _webui = sb.spawn_webui_no_secret();
    wait_reachable(&cli, &sb).await;
    let before = wait_secret_file(&sb).await;

    core.kill().await.expect("kill core");
    let cause = wait_unreachable_with_cause(&cli, &sb).await;
    assert!(!cause.is_empty(), "outage cause must be reported");

    let _core2 = sb.spawn_core_no_secret();
    // The restarted core must have minted a fresh secret (proves rotation
    // actually happened rather than the webui reconnecting to a stale key).
    let hint = sb.path.clone();
    let secret_path = sb.secret_file_path();
    let after = wait_for(
        "restarted core to mint a fresh secret",
        Duration::from_secs(15),
        &hint,
        move || {
            let path = secret_path.clone();
            let before = before.clone();
            Box::pin(async move {
                let raw = std::fs::read_to_string(&path).ok()?;
                let fresh = raw.trim().to_string();
                (!fresh.is_empty() && fresh != before).then_some(fresh)
            })
        },
    )
    .await;
    assert!(!after.is_empty());
    wait_reachable(&cli, &sb).await;
}

/// harden-core-channel-deployment §5.3 — supervision recovery journey
/// (narrows acceptance-ledger gap #3): `sebas run` supervises core + webui;
/// SIGKILL the supervised core child; the supervisor must restart it within
/// its restart delay and the webui must report reachable again.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn watchdog_supervision_restarts_killed_core() {
    let mut sb = Sandbox::new("testsuite_e2e", "supervision");
    sb.set_core_supervised();
    let cli = http_client();
    let mut run = sb.spawn_watchdog();
    let run_pid = run.id().expect("watchdog pid");
    wait_reachable(&cli, &sb).await;

    let core_pid = wait_supervised_core_pid(&sb, run_pid).await;
    unsafe { libc::kill(core_pid as libc::pid_t, libc::SIGKILL) };

    let cause = wait_unreachable_with_cause(&cli, &sb).await;
    assert!(!cause.is_empty(), "outage cause must be reported");

    // Supervisor restart (RESTART_DELAY 1s) + fresh boot: socket returns and
    // the supervised webui heals without any restart of its own.
    wait_reachable(&cli, &sb).await;
    assert!(
        sb.channel_path.exists(),
        "channel socket must exist again after supervisor restart"
    );
    assert_eq!(
        webui_healthy(&cli, &sb).await,
        Some(true),
        "webui must keep serving throughout supervision recovery"
    );
    // Tear down the whole supervised tree (kill_on_drop alone would orphan
    // the core/webui grandchildren).
    sb.reap_watchdog(&mut run).await;
}
