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
    http_client, post_json, wait_for, wait_router_addr, wait_reachable,
    wait_unreachable_with_cause, webui_healthy, Sandbox,
};

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

/// Find the direct child of `ppid` whose cmdline contains `needle`
/// (linux `/proc` walk; the supervised-recovery journey needs the core
/// CHILD pid, not the watchdog's). None while no such child is visible.
#[cfg(target_os = "linux")]
fn find_child_pid(ppid: u32, needle: &str) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == ppid {
            continue;
        }
        let Ok(cmd) = std::fs::read_to_string(entry.path().join("cmdline")) else {
            continue;
        };
        if !cmd.contains(needle) {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let is_child = status.lines().any(|l| {
            l.strip_prefix("PPid:")
                .map(|v| v.trim().parse::<u32>() == Ok(ppid))
                .unwrap_or(false)
        });
        if is_child {
            return Some(pid);
        }
    }
    None
}

/// 5.1 无密钥装配旅程：两个进程都不带 `SEBAS_CORE_SECRET` —— core 自动
/// 装配（生成密钥并写入 config 旁的 secret 文件）、webui 从文件发现密钥，
/// reachable 之后完成一次完整会话往返（事故回归：发现路径必须承载真实
/// 流量，而非仅握手成功）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn no_secret_assembly_end_to_end() {
    let sb = Sandbox::new("testsuite_e2e", "no-secret");
    let cli = http_client();
    let _core = sb.spawn_core_no_secret();
    let _webui = sb.spawn_webui_no_secret();
    wait_reachable(&cli, &sb).await;

    // 装配产物：secret 文件存在、64 位 hex、unix 上 0600。
    let secret_file = sb.secret_file();
    let secret = std::fs::read_to_string(&secret_file).expect("core.secret written at arm time");
    assert_eq!(
        secret.trim().len(),
        64,
        "generated key must be 64 hex chars"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secret_file)
            .expect("stat secret file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "secret file must be 0600, got {mode:o}");
    }

    // 完整会话往返（与带 env 用例同款断言）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "hello", "backend": "acp" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key in create response");
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

/// 5.2 密钥轮换自愈旅程：core 每次启动自动生成新钥并覆写 secret 文件；
/// kill → 同 config 重启（新钥）→ **不重启**的 webui 因每次连接重读文件
/// 而恢复 reachable；宕机窗口内 cause 如实上报。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn secret_rotation_self_heal_across_core_restart() {
    let sb = Sandbox::new("testsuite_e2e", "rotation");
    let cli = http_client();
    let mut core = sb.spawn_core_no_secret();
    let mut webui = sb.spawn_webui_no_secret();
    wait_reachable(&cli, &sb).await;
    let key1 = std::fs::read_to_string(sb.secret_file()).expect("secret file after first arm");

    core.kill().await.expect("kill core");
    let cause = wait_unreachable_with_cause(&cli, &sb).await;
    assert!(!cause.is_empty(), "downtime cause must be reported");

    // 同 config 重启：自动装配生成一把**新**钥（CSPRNG）。
    let _core2 = sb.spawn_core_no_secret();
    let hint = sb.path.clone();
    let secret_file = sb.secret_file();
    let key2 = wait_for(
        "rotated secret file",
        Duration::from_secs(15),
        &hint,
        move || {
            let path = secret_file.clone();
            let key1 = key1.clone();
            Box::pin(async move {
                let k2 = std::fs::read_to_string(&path).ok()?;
                let changed = !k2.trim().is_empty() && k2.trim() != key1.trim();
                changed.then_some(k2)
            })
        },
    )
    .await;
    assert_eq!(
        key2.trim().len(),
        64,
        "rotated key must be 64 hex chars"
    );

    // webui 从未重启：只能靠文件重读自愈。
    wait_reachable(&cli, &sb).await;
    assert!(
        webui.try_wait().expect("webui try_wait").is_none(),
        "the webui must not have exited during the rotation"
    );
}

/// 5.3 监督重启恢复旅程：watchdog 形态（`sebas run`）拉起 core + webui
/// 子进程；SIGKILL core **子进程** → supervisor 自动重启 → 未重启的
/// webui 子进程恢复 reachable（收窄账本缺口 #3 的监督形态证据）。
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn watchdog_supervised_core_recovery() {
    let sb = Sandbox::new("testsuite_e2e", "watchdog-recovery");
    sb.enable_supervised_core();
    // 控制socket（control.sock）按 XDG_RUNTIME_DIR 解析——钉进沙箱，绝不
    // 触碰宿主机上可能存在的真实实例控制面。
    let xdg_run = sb.path.join("xdg-run");
    std::fs::create_dir_all(&xdg_run).expect("mkdir xdg-run");
    let xdg = support::forward_slash(&xdg_run);
    let cfg = support::forward_slash(&sb.config_path);
    let cli = http_client();
    let mut watchdog = sb.spawn(
        &["run", "-c", &cfg],
        &sb.core_secret,
        &[("XDG_RUNTIME_DIR", &xdg)],
        &sb.core_log,
    );
    wait_reachable(&cli, &sb).await;

    let watchdog_pid = watchdog.id().expect("watchdog pid");
    let hint = sb.path.clone();
    let core_pid =
        wait_for("core child pid to appear", Duration::from_secs(15), &hint, move || {
            Box::pin(async move { find_child_pid(watchdog_pid, "core") })
        })
        .await;

    unsafe { libc::kill(core_pid as libc::pid_t, libc::SIGKILL) };
    wait_unreachable_with_cause(&cli, &sb).await;

    // supervisor 自动重启：新 core 子进程（pid 变化）出现。
    let new_pid = wait_for(
        "supervisor to respawn the core child",
        Duration::from_secs(45),
        &hint,
        move || {
            Box::pin(async move {
                find_child_pid(watchdog_pid, "core").filter(|p| *p != core_pid)
            })
        },
    )
    .await;
    assert_ne!(new_pid, core_pid, "supervisor must spawn a fresh core");

    // webui 子进程未重启即可恢复 reachable。
    wait_reachable(&cli, &sb).await;
    assert!(
        watchdog.try_wait().expect("watchdog try_wait").is_none(),
        "watchdog must stay up across the managed child crash"
    );
}
