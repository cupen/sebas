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
    Sandbox, http_client, post_json, wait_for, wait_reachable, wait_router_addr,
    wait_unreachable_with_cause, webui_healthy,
};

/// 从 core 日志里等出一次性 bootstrap 配对 token（只打印一次，读过就没了）。
async fn wait_bootstrap_token(sb: &Sandbox) -> String {
    let log = sb.core_log.clone();
    let hint = sb.path.clone();
    wait_for(
        "core 打印一次性 bootstrap 配对 token",
        Duration::from_secs(20),
        &hint,
        move || {
            let log = log.clone();
            Box::pin(async move {
                let text = std::fs::read_to_string(&log).ok()?;
                // 形如：… bootstrap 配对 token（只显示这一次，600 秒内有效）：<64hex>
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

/// 等某节点在**工作台可见的节点面**上进入给定状态（经 HTTP，不查内部结构）。
async fn wait_node_status(
    cli: &reqwest::Client,
    sb: &Sandbox,
    node_id: &str,
    want: &str,
) {
    let url = format!("{}/api/nodes", sb.webui_url());
    let want = want.to_string();
    let want_in_closure = want.clone();
    let node_id = node_id.to_string();
    let hint = sb.path.clone();
    let got = wait_for(
        &format!("节点 {node_id} 状态变为 {want}"),
        Duration::from_secs(30),
        &hint,
        move || {
            let cli = cli.clone();
            let url = url.clone();
            let want = want_in_closure.clone();
            let node_id = node_id.clone();
            Box::pin(async move {
                let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                let nodes = v.get("nodes")?.as_array()?.clone();
                let found = nodes
                    .iter()
                    .find(|n| n.get("id").and_then(|i| i.as_str()) == Some(node_id.as_str()))?;
                let status = found.get("status").and_then(|s| s.as_str())?;
                (status == want).then_some(status.to_string())
            })
        },
    )
    .await;
    assert_eq!(got, want);
}

/// 9.3 进程级 e2e：**core 与 node 是两个进程**。
///
/// 覆盖：一次性 bootstrap token 配对 → 节点在工作台可见 → 项目路径由**节点**判定
/// → 杀节点后如实离线 → 节点带原凭据重启后免刷新恢复（core 没动）→ 杀主控重启后
/// 节点自动重连并重新登记（主控视图重建，执行侧不受影响）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn remote_node_pairs_survives_node_and_core_restarts() {
    let sb = Sandbox::new("testsuite_e2e", "remote-node");
    sb.enable_node_link();
    let node_work = sb.node_work_dir();
    let cli = http_client();
    let mut core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 1) 配对：token 只打印一次，节点用它换长期凭据。
    let token = wait_bootstrap_token(&sb).await;
    let mut node = sb.spawn_node("itest-node", Some(&token));
    wait_node_status(&cli, &sb, "itest-node", "online").await;

    // 2) 远端项目注册：主控不 stat 这个路径，**节点**说了算。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({
            "path": node_work.to_string_lossy(),
            "node_id": "itest-node",
        }),
    )
    .await
    .expect("register remote project");
    assert_eq!(status, 201, "远端项目注册应由节点校验通过: {body}");

    // 3) 在**节点上**建会话并跑一轮：项目决定节点（d1），所以带上项目 id。
    let project_id = body["id"].as_str().expect("project id").to_string();
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "prompt": "hello",
            "agent": "echo",
            "project_id": project_id,
        }),
    )
    .await
    .expect("create remote session");
    assert_eq!(status, 201, "远端会话应建立成功: {body}");
    let key = body["key"].as_str().expect("session key").to_string();

    // 会话真的要落到节点上：投影把节点维度带到了工作台。
    let row = wait_for(
        "远端会话出现在工作台上并标注其节点",
        Duration::from_secs(20),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = format!("{}/api/sessions", sb.webui_url());
            let key = key.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                let key = key.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let rows = v.get("recent_sessions")?.as_array()?.clone();
                    rows.into_iter().find(|r| {
                        r.get("encoded_key").and_then(|k| k.as_str()) == Some(key.as_str())
                    })
                })
            }
        },
    )
    .await;
    assert_eq!(
        row["remote"]["node_id"].as_str(),
        Some("itest-node"),
        "远端行必须点名它跑在哪台机器上: {row}"
    );

    // 4) 一轮跑通：echo 执行体的应答经节点日志 → 事件流 → 工作台转写可见。
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let transcript = wait_for(
        "远端会话的转写出现 echo 应答",
        Duration::from_secs(20),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = detail_url.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let text = v
                        .get("entries")?
                        .as_array()?
                        .iter()
                        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
                        .collect::<Vec<_>>()
                        .join("");
                    text.contains("echo: hello").then_some(text)
                })
            }
        },
    )
    .await;
    assert!(transcript.contains("echo: hello"), "{transcript}");

    // 5) kill 主控重启：执行事实在节点上，重建视图后**对账补回**这一段。
    core.kill().await.expect("kill core");
    let _ = core.wait().await;
    let mut core = sb.spawn_core();
    wait_node_status(&cli, &sb, "itest-node", "online").await;
    let recovered = wait_for(
        "主控重启后对账补回远端会话的转写",
        Duration::from_secs(30),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = detail_url.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let text = v
                        .get("entries")?
                        .as_array()?
                        .iter()
                        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
                        .collect::<Vec<_>>()
                        .join("");
                    text.contains("echo: hello").then_some(text)
                })
            }
        },
    )
    .await;
    assert!(
        recovered.contains("echo: hello"),
        "主控重启后必须能从节点回拉出这轮事实（对账补齐）: {recovered}"
    );

    // 6) 悬空审批：`run:` 触发执行体里的受门控动作 → 节点停住、上报、等人批。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "run: echo gated" }),
    )
    .await
    .expect("send gated prompt");
    assert_eq!(status, 200, "{body}");

    // 「等待 ≠ 运行中」（spec 8.4）：有悬空审批的会话必须呈现为 waiting。
    let waiting = wait_for(
        "悬空审批把远端会话呈现为等待",
        Duration::from_secs(20),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = format!("{}/api/sessions", sb.webui_url());
            let key = key.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                let key = key.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let rows = v.get("recent_sessions")?.as_array()?.clone();
                    let row = rows
                        .into_iter()
                        .find(|r| r.get("encoded_key").and_then(|k| k.as_str()) == Some(key.as_str()))?;
                    let parked = row.get("remote")?.get("parked_approvals")?.as_u64()?;
                    (parked > 0).then_some(row)
                })
            }
        },
    )
    .await;
    assert_eq!(waiting["status_slug"].as_str(), Some("waiting"), "{waiting}");

    // 7) 杀节点：如实离线（不是"已终止"，也不是继续假装在线）。
    node.kill().await.expect("kill node");
    let _ = node.wait().await;
    wait_node_status(&cli, &sb, "itest-node", "offline").await;

    // 8) 节点带**原凭据**重启（不给 token）：主控进程没动，应免刷新恢复，
    //    且节点上那个会话还在（链路断了不等于会话没了）。
    let mut node = sb.spawn_node("itest-node", None);
    wait_node_status(&cli, &sb, "itest-node", "online").await;
    wait_for(
        "重启后的节点仍持有那个会话",
        Duration::from_secs(30),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = detail_url.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let text = v
                        .get("entries")?
                        .as_array()?
                        .iter()
                        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
                        .collect::<Vec<_>>()
                        .join("");
                    text.contains("echo: hello").then_some(text)
                })
            }
        },
    )
    .await;

    core.kill().await.ok();
    node.kill().await.ok();
}


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
    let healthy = wait_for(
        "webui /health ok",
        Duration::from_secs(10),
        &hint,
        move || {
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
        },
    )
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
        serde_json::json!({ "prompt": "hello", "agent": "claude" }),
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

    let transcript = detail["entries"]
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
    wait_for(
        "channel socket to appear",
        Duration::from_secs(15),
        &hint,
        move || {
            let socket = socket.clone();
            Box::pin(async move { socket.exists().then_some(()) })
        },
    )
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
        serde_json::json!({ "prompt": "hello", "agent": "claude" }),
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
    let transcript = detail["entries"]
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
    assert_eq!(key2.trim().len(), 64, "rotated key must be 64 hex chars");

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
    let core_pid = wait_for(
        "core child pid to appear",
        Duration::from_secs(15),
        &hint,
        move || Box::pin(async move { find_child_pid(watchdog_pid, "core") }),
    )
    .await;

    unsafe { libc::kill(core_pid as libc::pid_t, libc::SIGKILL) };
    wait_unreachable_with_cause(&cli, &sb).await;

    // supervisor 自动重启：新 core 子进程（pid 变化）出现。
    let new_pid = wait_for(
        "supervisor to respawn the core child",
        Duration::from_secs(45),
        &hint,
        move || {
            Box::pin(async move { find_child_pid(watchdog_pid, "core").filter(|p| *p != core_pid) })
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

/// workbench-turn-queue 2.3/5.2 + workbench-conversation-view BREAKING 更新：
/// 忙中提交的进程级时序与丢弃记账。fake-claude 经 `--slow-ms` 在内容帧与
/// result 之间停留——WORKING 窗口确定性的长：
/// 1. 忙中 POST message → 200 且 payload.pending 可见该提交（disposition
///    turn），条目序列里在跑回合的 prompt 不变（提交没有切开在跑输出，
///    也没有提前进 transcript——`kind="prompt"` 的最后一条仍是首提交）；
/// 2. 在跑回合结束后排队回合自动开轮：条目序列出现排队提交的 prompt 条目；
/// 3. close 带队列的会话 → 响应 `discarded_pending` 点名丢弃条数，会话移除。
/// （「输出不被切开」的结构性保证由引擎级单测
/// tests/continue_session_test.rs 断言；本用例证可观察面。）
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn turn_queue_timing_and_dropped_accounting() {
    let sb = Sandbox::new("testsuite_e2e", "turn-queue");
    // 慢档只取 400ms：driver 的 watchdog 每秒探测、1.5s 内须应答——更大的
    // 同步 sleep 会让 fake-claude 无法应答探测而被判挂起（会话被终结）。
    // 提交面用 "stream" 场景：5 帧 × 250ms 停顿，turn_active 窗口 ≈1.6s，
    // 且帧间 stdin 可读（watchdog 探测可被应答）。
    sb.slow_fake_agent(400);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 创建会话（首 prompt "stream" 触发流式场景）→ 等 WORKING 可见。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "stream", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();
    wait_for(
        "first turn content to stream",
        Duration::from_secs(40),
        &hint,
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
                    let streamed = v["entries"].as_array().is_some_and(|b| !b.is_empty());
                    streamed.then_some(v)
                })
            }
        },
    )
    .await;

    // 忙中提交 → 接受且入队：pending 携带该提交；条目序列里最后一条
    // prompt 仍是首提交（排队提交未开轮、绝不提前进 transcript）。
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "queued while busy" }),
    )
    .await
    .expect("submit while busy");
    assert_eq!(status, 200, "busy-time submission must be accepted: {resp}");
    let mid = cli
        .get(&detail_url)
        .send()
        .await
        .expect("detail mid-turn")
        .json::<serde_json::Value>()
        .await
        .expect("detail json");
    let pending = mid["pending"].as_array().expect("pending list mid-turn");
    assert_eq!(
        pending.len(),
        1,
        "the busy-time submission rides in pending: {mid}"
    );
    assert_eq!(pending[0]["text"], "queued while busy");
    assert_eq!(pending[0]["disposition"], "turn");
    let prompts: Vec<&str> = mid["entries"]
        .as_array()
        .expect("entries mid-turn")
        .iter()
        .filter(|e| e["kind"].as_str() == Some("prompt"))
        .filter_map(|e| e["content"].as_str())
        .collect();
    assert_eq!(
        prompts.last().copied(),
        Some("stream"),
        "the running turn must be untouched by the submission: {mid}"
    );
    assert!(
        !prompts.contains(&"queued while busy"),
        "a queued submission must not appear as a started turn: {prompts:?}"
    );

    // 排队回合自动开轮并完成：条目序列出现排队提交的 prompt 条目。
    wait_for(
        "queued turn to run and settle",
        Duration::from_secs(60),
        &hint,
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
                    let queued_started = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| {
                            e["kind"].as_str() == Some("prompt")
                                && e["content"].as_str() == Some("queued while busy")
                        })
                    });
                    let done = queued_started && v["status_slug"].as_str() == Some("done");
                    done.then_some(v)
                })
            }
        },
    )
    .await;

    // 时序断言：turn-1 的全部输出条目必须完整地排在 turn-2 输出之前——
    // 忙中提交绝不切开在跑回合的输出（可观察面证据；结构性保证由
    // tests/continue_session_test.rs 的引擎级单测承载）。
    let final_detail = cli
        .get(&detail_url)
        .send()
        .await
        .expect("final detail")
        .json::<serde_json::Value>()
        .await
        .expect("final json");
    let contents: Vec<String> = final_detail["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .filter_map(|b| b["content"].as_str().map(str::to_string))
        .collect();
    let last_chunk = contents
        .iter()
        .rposition(|c| c.contains("chunk"))
        .expect("turn-1 streamed chunks");
    let first_hello = contents
        .iter()
        .position(|c| c.contains("hello"))
        .expect("turn-2 reply");
    assert!(
        last_chunk < first_hello,
        "turn-2 output must not interleave turn-1 output: {contents:?}"
    );

    // close 记账：再造一个带队列的会话并关闭 → discarded_pending: 1。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "stream", "agent": "claude" }),
    )
    .await
    .expect("create second session");
    assert_eq!(status, 201, "{body}");
    let key2 = body["key"].as_str().expect("key").to_string();
    let detail2 = format!("{}/api/sessions/{key2}", sb.webui_url());
    wait_for(
        "second session content to stream",
        Duration::from_secs(25),
        &hint,
        {
            let cli = cli.clone();
            let url = detail2.clone();
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
                    v["entries"]
                        .as_array()
                        .is_some_and(|b| !b.is_empty())
                        .then_some(v)
                })
            }
        },
    )
    .await;
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key2}/message", sb.webui_url()),
        serde_json::json!({ "message": "will be dropped" }),
    )
    .await
    .expect("queue for close");
    assert_eq!(status, 200, "{resp}");
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key2}/close", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("close");
    assert_eq!(status, 200, "close: {resp}");
    assert_eq!(
        resp["discarded_pending"].as_u64(),
        Some(1),
        "close must name the dropped submission count: {resp}"
    );
}

/// make-core-own-provider-data 3.4 / router-admin-api「Configuration source」
/// 「card-edited provider reaches router」+「External change hot reload」
/// 「card edit hot-applies」：watchdog 形态下 router 以独立子进程运行
/// （watchdog 注入 SEBAS_CORE_SOCKET/SEBAS_CORE_SECRET），订阅 core 通道。
/// 经 webui BFF 写入 core 状态库的 provider 在 router 不重启、不写任何
/// provider 文件的情况下变为可路由——router 只是 core 数据的只读消费者。
/// （此前的全部测试要么用 spawn 前写好的 seed 文件，要么是 core/webui 单侧
/// 闭环；router 的通道订阅投影 reload_from_channel 在任何层都无进程级覆盖。）
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn core_owned_provider_reaches_router_without_restart() {
    let sb = Sandbox::new("testsuite_e2e", "provider-hotswap");
    // 固定默认端口 8787 会让并行用例互踩——每例钉一个空闲端口。
    let router_port = support::free_port();
    sb.set_router_listen(router_port);

    // 本地假上游（anthropic 协议应答，记录被问到的 model）——绝不连外网。
    let asked = Arc::new(tokio::sync::Mutex::new(None::<String>));
    let stub = support::spawn_stub_upstream(asked.clone()).await;

    // watchdog 形态：`run --debug` 强制 router 子服务上线。
    let xdg_run = sb.path.join("xdg-run");
    std::fs::create_dir_all(&xdg_run).expect("mkdir xdg-run");
    let xdg = support::forward_slash(&xdg_run);
    let cfg = support::forward_slash(&sb.config_path);
    let cli = http_client();
    let mut watchdog = sb.spawn(
        &["run", "-c", &cfg, "--debug"],
        &sb.core_secret,
        &[("XDG_RUNTIME_DIR", &xdg)],
        &sb.core_log,
    );
    wait_reachable(&cli, &sb).await;
    let watchdog_pid = watchdog.id().expect("watchdog pid");

    // router 子进程出现并解析监听地址（子进程 stdout/stderr inherit → 日志）。
    let hint = sb.path.clone();
    let log = sb.core_log.clone();
    let (router_pid, router_url) = wait_for(
        "router child addr in log",
        Duration::from_secs(30),
        &hint,
        move || {
            let log = log.clone();
            Box::pin(async move {
                let pid = find_child_pid(watchdog_pid, "router")?;
                let text = std::fs::read_to_string(&log).ok()?;
                for line in text.lines().rev() {
                    if line.contains("router listening")
                        && let Some(idx) = line.find("addr=")
                        && let Ok(addr) = line[idx + 5..]
                            .split_whitespace()
                            .next()?
                            .parse::<std::net::SocketAddr>()
                    {
                        return Some((pid, format!("http://{addr}")));
                    }
                }
                None
            })
        },
    )
    .await;
    assert_eq!(
        router_url,
        format!("http://127.0.0.1:{router_port}"),
        "router child must honor the pinned [router] listen"
    );

    // 经 webui BFF 建 provider：写入 core 状态库（core 是唯一写者）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/router/api/providers", sb.webui_url()),
        serde_json::json!({
            "name": "stub",
            "protocol": "anthropic",
            "base_url_anthropic": format!("http://127.0.0.1:{stub}"),
            "api_key": "sk-stub-e2e"
        }),
    )
    .await
    .expect("create provider via webui BFF");
    assert_eq!(status, 201, "provider create: {body}");

    // 订阅驱动：不重启任何进程，router 按新 provider 路由（改变通知 →
    // StateSnapshot 拉取 → 热交换）。轮询直到首次命中。
    wait_for(
        "router routes the core-stored provider",
        Duration::from_secs(60),
        &hint,
        move || {
            let cli = cli.clone();
            let url = format!("{router_url}/v1/messages");
            Box::pin(async move {
                let Ok((status, body)) = post_json(
                    &cli,
                    &url,
                    serde_json::json!({
                        "model": "stub/stub-model",
                        "max_tokens": 16,
                        "messages": [{ "role": "user", "content": "hi" }]
                    }),
                )
                .await
                else {
                    return None;
                };
                (status == 200 && body["id"] == "msg_stub").then_some(body)
            })
        },
    )
    .await;
    assert_eq!(
        asked.lock().await.as_deref(),
        Some("stub-model"),
        "upstream must receive the namespace-rest model id"
    );

    // router 只是只读消费者：provider 文件从未被写（core 状态库是唯一真源）。
    assert!(
        !sb.path.join("providers.json").exists(),
        "no provider file may be written in the core-owned topology"
    );
    // router 子进程全程未重启（同一 pid），且 pid 确是 router 子进程
    // （cmdline 首参校验——find_child_pid 按 cmdline 子串匹配，先自证锚点）。
    let pid_now = find_child_pid(watchdog_pid, "router");
    assert_eq!(
        pid_now,
        Some(router_pid),
        "router must not restart for a provider change to take effect"
    );
    if let Some(pid) = pid_now {
        let cmd = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
            .expect("read router child cmdline");
        let mut argv = cmd.split('\0');
        let _exe = argv.next();
        assert!(
            argv.next() == Some("router"),
            "matched child must be the router subprocess, got cmdline {cmd:?}"
        );
    }
    assert!(
        watchdog.try_wait().expect("watchdog try_wait").is_none(),
        "watchdog must stay up"
    );
}

/// （add-agent-mode-selection）mode 透传：带 mode 的创建把映射后的
/// `--permission-mode` 写进子进程 argv；缺省 mode 的对照会话不带该参数；
/// 未知 mode 创建 400。数据源是 fake-claude journal（argv/meta），全程零真
/// 模型调用。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn mode_threads_to_agent_argv() {
    let sb = Sandbox::new("testsuite_e2e", "mode-argv");
    let journal = sb.journal_fake_agent();
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 对照会话：不带 mode → argv 无 --permission-mode。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create default session");
    assert_eq!(status, 201, "create default session: {body}");

    // mode=allow → argv 含 --permission-mode bypassPermissions。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "hello", "agent": "claude", "mode": "allow" }),
    )
    .await
    .expect("create allow session");
    assert_eq!(status, 201, "create allow session: {body}");

    // 未知 mode → 400（词汇校验，不静默降级）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "x", "agent": "claude", "mode": "plan" }),
    )
    .await
    .expect("create unknown-mode session");
    assert_eq!(status, 400, "unknown mode must be rejected: {body}");

    // 两个子进程各追加一条 argv meta。
    wait_for(
        "both fake-claude turns to finish (journal has 2 argv metas)",
        Duration::from_secs(30),
        &sb.path.clone(),
        {
            let journal = journal.clone();
            move || {
                let journal = journal.clone();
                Box::pin(async move {
                    let Ok(content) = std::fs::read_to_string(&journal) else {
                        return None;
                    };
                    let metas = content
                        .lines()
                        .filter(|l| l.contains("\"dir\":\"meta\""))
                        .count();
                    (metas >= 2).then_some(())
                })
            }
        },
    )
    .await;

    let content =
        std::fs::read_to_string(sb.path.join("fake-claude-journal.jsonl")).expect("journal");
    let argvs: Vec<Vec<String>> = content
        .lines()
        .filter(|l| l.contains("\"dir\":\"meta\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("journal meta parses");
            v["msg"]["argv"]
                .as_array()
                .expect("argv array")
                .iter()
                .map(|a| a.as_str().expect("argv str").to_string())
                .collect()
        })
        .collect();
    assert_eq!(argvs.len(), 2, "two child spawns journaled: {argvs:?}");

    let with_flag = argvs
        .iter()
        .find(|a| a.iter().any(|t| t == "--permission-mode"))
        .expect("allow session must carry --permission-mode");
    let idx = with_flag
        .iter()
        .position(|t| t == "--permission-mode")
        .unwrap();
    assert_eq!(
        with_flag.get(idx + 1).map(String::as_str),
        Some("bypassPermissions"),
        "allow maps to bypassPermissions on the child argv"
    );
    assert!(
        argvs
            .iter()
            .any(|a| !a.iter().any(|t| t == "--permission-mode")),
        "default session must not carry --permission-mode"
    );
}

/// （add-agent-mode-selection）中途切换：POST /api/sessions/{key}/mode 把
/// 期望值送达运行中的 fake-claude（journal 记录运行时 set_permission_mode），
/// 快照 desired/effective 跟随更新；切到 allow 后 `perm` 场景免审批直接
/// 完成（行为级对照：ask 下同场景会停在审批等待）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn mode_mid_session_switch() {
    let sb = Sandbox::new("testsuite_e2e", "mode-switch");
    let journal = sb.journal_fake_agent();
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 等首回合完成。
    wait_for(
        "first turn Done",
        Duration::from_secs(25),
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
                    let done = v["status_slug"].as_str() == Some("done")
                        || v["status"]
                            .as_str()
                            .is_some_and(|s| s.eq_ignore_ascii_case("done"));
                    done.then_some(())
                })
            }
        },
    )
    .await;

    // 切到 allow：命令送达 = 200；执行体接受经 ModeChanged 反馈。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/mode", sb.webui_url()),
        serde_json::json!({ "mode": "allow" }),
    )
    .await
    .expect("switch mode");
    assert_eq!(status, 200, "mode switch: {body}");

    // journal 记录运行时切换（SDK set_permission_mode → "bypassPermissions"）。
    wait_for(
        "runtime mode_change journaled",
        Duration::from_secs(10),
        &sb.path.clone(),
        {
            let journal = journal.clone();
            move || {
                let journal = journal.clone();
                Box::pin(async move {
                    let Ok(content) = std::fs::read_to_string(&journal) else {
                        return None;
                    };
                    content
                        .lines()
                        .any(|l| l.contains("mode_change") && l.contains("bypassPermissions"))
                        .then_some(())
                })
            }
        },
    )
    .await;

    // 快照：desired=allow 且 effective 落定为 allow（ModeChanged → 映射）。
    wait_for(
        "snapshot reflects effective mode",
        Duration::from_secs(10),
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
                    let ok = v["desired_mode"].as_str() == Some("allow")
                        && v["effective_mode"].as_str() == Some("allow");
                    ok.then_some(())
                })
            }
        },
    )
    .await;

    // 行为级：allow 模式下 perm 场景免审批直接完成。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "perm" }),
    )
    .await
    .expect("send perm");
    assert_eq!(status, 200, "send perm: {body}");
    let detail = wait_for(
        "perm turn completes without gating",
        Duration::from_secs(25),
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
                    let done = v["status_slug"].as_str() == Some("done")
                        || v["status"]
                            .as_str()
                            .is_some_and(|s| s.eq_ignore_ascii_case("done"));
                    done.then_some(v)
                })
            }
        },
    )
    .await;
    let transcript = detail["entries"]
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
        transcript.contains("perm done"),
        "allow session must run the gated tool without approval, got: {transcript:?}"
    );
}
