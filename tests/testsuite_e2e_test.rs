//! Process-level e2e for the core flows in the detached (watchdog) topology:
//! a real core child (`sebas core`) plus a standalone `sebas webui` process
//! connected through the core session channel; journeys that need the gateway
//! additionally spawn an independent `sebas router --config … --debug` child
//! （unify-router-process-shape D5：router 只以独立进程运行）.
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

use std::sync::Arc;
use std::time::Duration;

mod support;

use support::{
    Sandbox, free_port, http_client, next_ws_frame, post_json, scene_project_id,
    spawn_sse_stub_upstream, wait_for, wait_reachable, wait_router_addr,
    wait_unreachable_with_cause, webui_healthy, ws_connect, WsStream,
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
                text.split("有效）：").nth(1).and_then(|rest| {
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
async fn wait_node_status(cli: &reqwest::Client, sb: &Sandbox, node_id: &str, want: &str) {
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
                let v = cli
                    .get(&url)
                    .send()
                    .await
                    .ok()?
                    .json::<serde_json::Value>()
                    .await
                    .ok()?;
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
                    let v = cli
                        .get(&url)
                        .send()
                        .await
                        .ok()?
                        .json::<serde_json::Value>()
                        .await
                        .ok()?;
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
                    let v = cli
                        .get(&url)
                        .send()
                        .await
                        .ok()?
                        .json::<serde_json::Value>()
                        .await
                        .ok()?;
                    let rows = v.get("recent_sessions")?.as_array()?.clone();
                    let row = rows.into_iter().find(|r| {
                        r.get("encoded_key").and_then(|k| k.as_str()) == Some(key.as_str())
                    })?;
                    let parked = row.get("remote")?.get("parked_approvals")?.as_u64()?;
                    (parked > 0).then_some(row)
                })
            }
        },
    )
    .await;
    assert_eq!(
        waiting["status_slug"].as_str(),
        Some("waiting"),
        "{waiting}"
    );

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

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
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

// ---- session-parallel-liveness-and-unread-polish 1.2：双会话并行 spawn ----

/// Two 0-turn placeholder sessions spawn concurrently: while the first
/// session's child is mid-handshake (fake-claude `--delay-init-ms 2500`),
/// the second session's spawn instruction SHALL NOT queue behind the first
/// handshake. The ordering that proves it: B (instant "hello" turn) finishes
/// BEFORE A (800ms "stream" turn after an identical handshake) — serialized
/// spawn flips the order (B's handshake only starts after A activates).
///
/// Fix basis: `dispatch_out_without_feishu` spawns a dedicated task per
/// WebSpawn (`src/dispatch.rs`, design D1 candidate A). Before the fix this
/// journey takes ≥ 2 handshakes serialised; after it the two handshakes
/// overlap.
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn two_sessions_spawn_and_turn_concurrently() {
    let sb = Sandbox::new("testsuite_e2e", "parallel-spawn");
    // Stretch the handshake so the two spawn instructions measurably overlap:
    // every fake-claude child answers initialize only after 2.5s.
    sb.append_acp_args(&["--delay-init-ms", "2500"]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // Two first messages back-to-back: both sessions enter the spawn window.
    let project_id = scene_project_id(&cli, &sb).await;
    let (status_a, body_a) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stream", "agent": "claude" }),
    )
    .await
    .expect("create session A");
    assert_eq!(status_a, 201, "create A: {body_a}");
    let key_a = body_a["key"].as_str().expect("key A").to_string();

    let project_id = scene_project_id(&cli, &sb).await;
    let (status_b, body_b) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session B");
    assert_eq!(status_b, 201, "create B: {body_b}");
    let key_b = body_b["key"].as_str().expect("key B").to_string();

    // B 完成时（hello 回合零等待）A 必须**还没**完成（stream 含 800ms 拖
    // 尾）。串行 spawn 下 B 的握手要等 A 激活 → B done ≈ 5s > A done ≈ 3.3s；
    // 并行 spawn 下两握手重叠 → B done ≈ 2.6s < A done ≈ 3.3s。
    async fn fetch_detail(cli: &reqwest::Client, url: &str) -> Option<serde_json::Value> {
        cli.get(url).send().await.ok()?.json().await.ok()
    }
    fn is_done(v: &serde_json::Value) -> bool {
        v["status_slug"].as_str() == Some("done")
            || v["status"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("done"))
    }

    let url_a = format!("{}/api/sessions/{key_a}", sb.webui_url());
    let url_b = format!("{}/api/sessions/{key_b}", sb.webui_url());
    let hint = sb.path.clone();
    let b_done_before_a = wait_for(
        "B finishes its turn while A is still working",
        Duration::from_secs(20),
        &hint,
        {
            let cli = cli.clone();
            let url_a = url_a.clone();
            let url_b = url_b.clone();
            move || {
                let cli = cli.clone();
                let url_a = url_a.clone();
                let url_b = url_b.clone();
                Box::pin(async move {
                    let b = fetch_detail(&cli, &url_b).await?;
                    if !is_done(&b) {
                        return None;
                    }
                    let a = fetch_detail(&cli, &url_a).await?;
                    // B done 且 A 未 done = 两会话确实并行活着。
                    (!is_done(&a)).then_some(true)
                })
            }
        },
    )
    .await;
    assert!(
        b_done_before_a,
        "B (instant hello turn) must finish before A (2.5s handshake + 800ms \
         stream); B only finishing after A means the second spawn waited for \
         the first session — `core.log` in the kept sandbox dir has the \
         dispatch timeline"
    );

    // B 完成后 A 也必须顺利完成自己的回合（两个活子进程都走到了 done）。
    let hint = sb.path.clone();
    let a_done = wait_for(
        "A finishes its stream turn",
        Duration::from_secs(20),
        &hint,
        {
            let cli = cli.clone();
            let url_a = url_a.clone();
            move || {
                let cli = cli.clone();
                let url_a = url_a.clone();
                Box::pin(async move { fetch_detail(&cli, &url_a).await.filter(is_done) })
            }
        },
    )
    .await;
    assert!(is_done(&a_done), "A must complete its stream turn too");
}

// ---- workbench-interaction-polish 6.1：cancel 链路（BFF → core channel）----

/// Cancel 链路的类型化拒绝：未知 key 404；已知但空闲（无在飞 turn）的会话
/// 409——「无事可取消」不再伪造成功。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn cancel_typed_rejections_over_webui_http() {
    let sb = Sandbox::new("testsuite_e2e", "cancel-reject");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 未知 key → 404 typed rejection。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/web%00ghost/cancel", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("cancel unknown");
    assert_eq!(status, 404, "unknown key must 404: {body}");

    // 已知会话、无在飞 turn（首个 turn 已收敛）→ 409 空闲。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_for(
        "session turn to reach Done",
        Duration::from_secs(25),
        &sb.path.clone(),
        || {
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
                (v["status_slug"].as_str() == Some("done")).then_some(v)
            })
        },
    )
    .await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/cancel", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("cancel idle");
    assert_eq!(status, 409, "idle session must 409: {body}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("空闲"),
        "{body}"
    );

    // 会话不受拒绝影响：仍然可继续对话（补一条消息也收敛）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("follow-up after idle cancel");
    assert_eq!(status, 200, "{body}");
}

/// 真实中断路径（受 stub 限制如实标注）：fake-claude 桩对 control_request
/// interrupt 的语义是「turn 以错误结果收尾 + 子进程退出 → 驱动 respawn
/// with resume」。这里验证 cancel 链路端到端打通——200、turn 离开
/// working、会话存活可继续——「真模型的中断体验」另需真实凭据，沙箱只能
/// 证到桩级（AGENTS.md 诚实边界）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn cancel_interrupts_in_flight_turn_over_webui_http() {
    let sb = Sandbox::new("testsuite_e2e", "cancel-interrupt");
    // 每个 turn 在飞 ≈2s（5 帧 × 250ms + slow-ms 800），给取消留窗口。
    sb.slow_fake_agent(800);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stream", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "{body}");
    let key = body["key"].as_str().expect("key").to_string();

    // 等 turn 开轮（WORKING）再取消。
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_for(
        "session turn to reach Working",
        Duration::from_secs(25),
        &sb.path.clone(),
        || {
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
                (v["status_slug"].as_str() == Some("working")).then_some(v)
            })
        },
    )
    .await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/cancel", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("cancel in-flight");
    assert_eq!(status, 200, "cancel must be accepted: {body}");
    assert_eq!(body["status"].as_str(), Some("cancelled"), "{body}");

    // turn 离开 working；会话保留且可继续（child 退出后驱动 respawn）。
    wait_for(
        "session turn to leave Working",
        Duration::from_secs(25),
        &sb.path.clone(),
        || {
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
                (v["status_slug"].as_str() != Some("working")).then_some(v)
            })
        },
    )
    .await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("follow-up after cancel");
    assert_eq!(status, 200, "session survives a cancel: {body}");
}

/// core 不可达时 cancel 的诚实退化：503，绝不伪造成功。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn cancel_without_core_answers_503() {
    let sb = Sandbox::new("testsuite_e2e", "cancel-503");
    let cli = http_client();
    let mut core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 先建一个会话（idle 即可——503 来自通道不可达，先于 idle 判定）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "{body}");
    let key = body["key"].as_str().expect("key").to_string();

    // core 下线。
    core.kill().await.expect("kill core");
    wait_unreachable_with_cause(&cli, &sb).await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/cancel", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("cancel without core");
    assert_eq!(status, 503, "unreachable core must answer 503: {body}");
}

/// The built-in debug router answers `model = "test"` over /v1/messages.
/// （独立 router 子进程：`sebas router --config <沙箱配置> --debug`。）
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn router_debug_provider_serves_messages() {
    let sb = Sandbox::new("testsuite_e2e", "router");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _router = sb.spawn_router_debug();

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

// ---------------------------------------------------------------------------
// fake-provider-upstream：本地 Anthropic 线协议假上游（零 token）接线
// ---------------------------------------------------------------------------

/// 假上游内置规则的确定性文案（`sebas_router::fake_provider` 常量；测试侧
/// 独立钉死字面量，避免断言随实现漂移而静默放宽）。
const FAKE_PLAIN_TEXT: &str = "fake-provider: no tools requested";
const FAKE_FINAL_TEXT: &str = "fake-provider: tool loop complete";
/// `[provider.fake]` 的上游哑 key（sandbox 模板）——透传断言的期望值。
const FAKE_UPSTREAM_KEY: &str = "sk-fake-upstream-dummy";
/// 下游 key：绝不能出现在 fake 的 journal 里。
const DOWNSTREAM_KEY: &str = "sk-downstream-must-not-leak";

/// 读 NDJSON 行（journal / usage），空行跳过。
fn read_jsonl(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// fake-provider-upstream 3.1：`spawn_fake_provider` helper 冒烟——spawn →
/// 拨号 200 → journal 落一条 → 拆卸后端口释放。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn fake_provider_spawn_helper_smoke() {
    let sb = Sandbox::new("testsuite_e2e", "fake-provider-helper");
    let cli = http_client();
    let mut fake = sb.spawn_fake_provider(None).await;
    assert!(fake.port > 0, "probed random port must be non-zero");
    assert!(
        fake.base_url.starts_with("http://127.0.0.1:"),
        "loopback base url: {}",
        fake.base_url
    );

    let (status, body) = post_json(
        &cli,
        &format!("{}/v1/messages", fake.base_url),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await
    .expect("dial fake provider");
    assert_eq!(status, 200, "fake /v1/messages: {body}");
    assert_eq!(body["content"][0]["text"], FAKE_PLAIN_TEXT);
    assert_eq!(
        read_jsonl(&fake.journal).len(),
        1,
        "one dial → exactly one journal line"
    );

    // 拆卸（等价 SandboxDir Drop 的进程收割）：端口必须释放。
    fake.child.kill().await.expect("kill fake provider");
    let _ = fake.child.wait().await;
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", fake.port)
        .parse()
        .expect("addr");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(_) => break,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("port {} not released after teardown: {e}", fake.port),
        }
    }
}

/// fake-provider-upstream 3.3：provider 透传全链路 journey。
///
/// 本地 fake 上游 + sandbox `[provider.fake]`（namespace `fake/fake-model`）+
/// **非 debug** 独立 router → 非流式（应答 + usage 落账）/ 流式（SSE 逐事件
/// 透传）/ journal 离线断言（上游 key 注入、下游 key 与 hop-by-hop 不泄漏）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn fake_provider_passthrough_journey() {
    let sb = Sandbox::new("testsuite_e2e", "fake-passthrough");
    let cli = http_client();
    let fake = sb.spawn_fake_provider(None).await;
    sb.enable_fake_provider(&fake.base_url);
    let _core = sb.spawn_core();
    let _router = sb.spawn_router(); // 非 debug：走真透传面（无内置 test provider）
    let router = wait_router_addr(&sb).await;
    let url = format!("{router}/v1/messages");
    let payload = serde_json::json!({
        "model": "fake/fake-model",
        "max_tokens": 32,
        "messages": [{ "role": "user", "content": "hello" }]
    });

    // --- 非流式：fake 的确定性内容 + router usage 落账 ---
    let resp = cli
        .post(&url)
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .header("x-api-key", DOWNSTREAM_KEY)
        .body(payload.to_string())
        .send()
        .await
        .expect("non-stream passthrough");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["id"], "msg_fake_builtin", "fake reply: {body}");
    assert_eq!(body["content"][0]["text"], FAKE_PLAIN_TEXT);
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["usage"]["input_tokens"].as_u64().unwrap_or(0) > 0);

    // usage 结算：非零 input/output + provider 名 + model rename 记录。
    let usage_path = sb.path.join("router-usage.jsonl");
    let hint = sb.path.clone();
    let fake_usage = {
        let usage_path = usage_path.clone();
        wait_for(
            "router usage record for provider fake",
            Duration::from_secs(15),
            &hint,
            move || {
                let usage_path = usage_path.clone();
                Box::pin(async move {
                    read_jsonl(&usage_path)
                        .into_iter()
                        .find(|r| r["provider"] == "fake" && r["status"] == 200)
                })
            },
        )
        .await
    };
    assert_eq!(fake_usage["model"], "fake/fake-model");
    assert_eq!(fake_usage["upstream_model"], "fake-model");
    assert!(
        fake_usage["input_tokens"].as_u64().unwrap_or(0) > 0
            && fake_usage["output_tokens"].as_u64().unwrap_or(0) > 0,
        "non-zero usage must settle: {fake_usage}"
    );

    // --- 流式：SSE 完整事件序列透传，文本与非流式一致 ---
    let mut streaming = payload.clone();
    streaming["stream"] = serde_json::json!(true);
    let resp = cli
        .post(&url)
        .header("content-type", "application/json")
        .body(streaming.to_string())
        .send()
        .await
        .expect("stream passthrough");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers().get("content-type").map(|v| v.to_str().unwrap()),
        Some("text/event-stream")
    );
    let sse = resp.text().await.expect("sse body");
    for event in [
        "event: message_start",
        "event: content_block_start",
        "event: content_block_delta",
        "event: content_block_stop",
        "event: message_delta",
        "event: message_stop",
    ] {
        assert!(sse.contains(event), "missing {event} in SSE:\n{sse}");
    }
    assert!(
        sse.contains(FAKE_PLAIN_TEXT),
        "SSE text must match the non-stream reply:\n{sse}"
    );

    // 流式也落账（SseUsageParser 从 message_start/message_delta 取 usage）。
    let usage_path2 = usage_path.clone();
    let hint2 = sb.path.clone();
    wait_for(
        "second usage record (stream settled)",
        Duration::from_secs(15),
        &hint2,
        move || {
            let usage_path2 = usage_path2.clone();
            Box::pin(async move {
                let fake_records = read_jsonl(&usage_path2)
                    .into_iter()
                    .filter(|r| r["provider"] == "fake")
                    .count();
                (fake_records >= 2).then_some(())
            })
        },
    )
    .await;

    // --- journal 离线断言：上游 key 注入 / 下游 key 与 hop-by-hop 不泄漏 ---
    let journal = read_jsonl(&fake.journal);
    assert_eq!(
        journal.len(),
        2,
        "router forwarded exactly two requests: {journal:?}"
    );
    let raw = std::fs::read_to_string(&fake.journal).expect("journal text");
    assert!(
        !raw.contains(DOWNSTREAM_KEY),
        "downstream key must never reach the upstream:\n{raw}"
    );
    let non_stream = journal
        .iter()
        .find(|l| l["body"]["stream"].as_bool() != Some(true))
        .expect("non-stream journal line");
    let headers = non_stream["headers"]
        .as_object()
        .expect("journal headers object");
    assert_eq!(
        headers.get("x-api-key").and_then(|v| v.as_str()),
        Some(FAKE_UPSTREAM_KEY),
        "upstream key must be injected: {headers:?}"
    );
    assert!(
        !headers.contains_key("authorization"),
        "downstream auth header must be stripped: {headers:?}"
    );
    for hop in [
        "connection",
        "transfer-encoding",
        "keep-alive",
        "te",
        "trailer",
        "upgrade",
    ] {
        assert!(
            !headers.contains_key(hop),
            "{hop} is hop-by-hop and must not be forwarded: {headers:?}"
        );
    }
    assert_eq!(
        non_stream["body"]["model"], "fake-model",
        "namespace rest + rename lands upstream: {non_stream}"
    );
    assert_eq!(non_stream["body"]["messages"][0]["content"], "hello");
}

/// fake-provider-upstream 3.4：确定性限流/用量——fake 秒回消除真实上游网络
/// 延迟抖动，token bucket 的越界 429 精确可复现，且越界请求不外呼。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn fake_provider_deterministic_rate_limit() {
    let sb = Sandbox::new("testsuite_e2e", "fake-rate-limit");
    let cli = http_client();
    let fake = sb.spawn_fake_provider(None).await;
    sb.enable_fake_provider(&fake.base_url);
    // capacity=2 + 极慢 refill：测试窗口内不自动补充（消除时序抖动）。
    sb.set_router_rate_limit(2, 0.0001);
    let _router = sb.spawn_router();
    let router = wait_router_addr(&sb).await;
    let url = format!("{router}/v1/messages");
    let payload = serde_json::json!({
        "model": "fake/fake-model",
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "hi" }]
    })
    .to_string();

    let mut statuses = Vec::new();
    let mut over_body = serde_json::Value::Null;
    for _ in 0..3 {
        let resp = cli
            .post(&url)
            .header("content-type", "application/json")
            .body(payload.clone())
            .send()
            .await
            .expect("POST /v1/messages");
        let status = resp.status().as_u16();
        if status == 429 {
            over_body = resp.json().await.expect("429 body");
        } else {
            let _ = resp.text().await;
        }
        statuses.push(status);
    }
    assert_eq!(
        statuses,
        vec![200, 200, 429],
        "bucket capacity 2 drained deterministically (fake answers instantly)"
    );
    assert_eq!(over_body["type"], "error");
    assert_eq!(over_body["error"]["type"], "rate_limit_error");

    // 无外呼浪费：只有桶内两个请求到达 fake。
    let journal = read_jsonl(&fake.journal);
    assert_eq!(
        journal.len(),
        2,
        "over-capacity request must not reach the upstream: {journal:?}"
    );
}

/// 真 claude-code 二进制：`SEBAS_TEST_CLAUDE_BIN` 优先，PATH 兜底；缺席 → None。
fn find_claude_bin() -> Option<String> {
    if let Ok(p) = std::env::var("SEBAS_TEST_CLAUDE_BIN")
        && !p.trim().is_empty()
    {
        let path = std::path::PathBuf::from(&p);
        if path.is_file() {
            return Some(p);
        }
        eprintln!(
            "[skip] SEBAS_TEST_CLAUDE_BIN={p} is not an existing file — ignoring it"
        );
    }
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// fake-provider-upstream 3.5：agent-loop journey（零 token）。
///
/// 真 claude-code 作 ACP 执行体、模型请求打到本地 fake 上游：消息 →
/// tool_use → 工具执行 → tool_result → 终文本 → 会话 Done。claude-code
/// 缺席（`SEBAS_TEST_CLAUDE_BIN` / PATH 都没有）时输出原因并跳过，不判失败。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn agent_loop_journey_claude_over_fake_upstream() {
    let Some(claude_bin) = find_claude_bin() else {
        eprintln!(
            "[skip] agent_loop_journey_claude_over_fake_upstream: no claude-code binary \
             (set SEBAS_TEST_CLAUDE_BIN or put `claude` on PATH); journey ready"
        );
        return;
    };

    let sb = Sandbox::new("testsuite_e2e", "agent-loop");
    let cli = http_client();
    let fake = sb.spawn_fake_provider(None).await;
    sb.set_agent_claude_path(&claude_bin);
    // claude-code 子进程继承 core 的 env：ProviderResolution::Off 不注入任何
    // provider env，这里直接把假上游地址+哑 token 塞给它（零登录、零 token；
    // 真实的 claude 登录态与真实上游完全不参与）。
    let _core = sb.spawn_core_extra(&[
        ("ANTHROPIC_BASE_URL", fake.base_url.as_str()),
        ("ANTHROPIC_AUTH_TOKEN", "sk-fake-agent-loop"),
    ]);
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "Use the Read tool on the file SEBAS_FAKE_TOOL_LOOP, then report what happened.", "agent": "claude", "mode": "allow" }),
    )
    .await
    .expect("create ACP session");
    assert_eq!(status, 201, "session create: {body}");
    let key = body["key"].as_str().expect("session key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 真 claude-code 冷启动 + 两轮假上游应答；预算放宽但有界。
    let hint = sb.path.clone();
    let detail = wait_for(
        "agent-loop turn to reach Done",
        Duration::from_secs(180),
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
                    (v["status_slug"].as_str() == Some("done")).then_some(v)
                })
            }
        },
    )
    .await;

    let entries = detail["entries"].as_array().cloned().unwrap_or_default();
    let transcript: String = entries
        .iter()
        .filter_map(|e| e["content"].as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        transcript.contains(FAKE_FINAL_TEXT),
        "turn must carry the fake upstream's final text, got: {transcript:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["element_type"].as_str() == Some("tool")),
        "tool execution trace must be visible in the turn: {transcript:?}"
    );

    // agent 工具环确实闭合：带工具表的回合 ≥2 —— 首轮无 tool_result（要求
    // tool_use），次轮带 tool_result（要求终文本）。claude-code 在会话创建时
    // 还会先打一轮无 tools 的标题生成请求（也是 fake 的活体证据）。
    let journal = read_jsonl(&fake.journal);
    let loop_turns: Vec<&serde_json::Value> = journal
        .iter()
        .filter(|l| {
            l["body"]["tools"]
                .as_array()
                .is_some_and(|t| !t.is_empty())
        })
        .collect();
    assert!(
        loop_turns.len() >= 2,
        "tool loop needs ≥2 tool-carrying upstream turns, got {journal:?}"
    );
    let first = loop_turns[0];
    assert!(
        first["body"]["model"].is_string(),
        "real claude-code sends a model id: {first}"
    );
    let has_tool_result = |l: &serde_json::Value| {
        l["body"]["messages"].as_array().is_some_and(|msgs| {
            msgs.iter().any(|m| {
                m["content"].as_array().is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|b| b["type"].as_str() == Some("tool_result"))
                })
            })
        })
    };
    assert!(
        !has_tool_result(first),
        "the first tool-carrying turn has no tool_result yet: {first}"
    );
    let last = loop_turns.last().expect("non-empty loop turns");
    assert!(
        has_tool_result(last),
        "the closing turn must carry a tool_result: {last}"
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
///
/// Windows/macOS have no `/proc`; the provider-hotswap journey keeps its
/// process-tree assertions linux-gated (D1) so its portable body still runs
/// there. Socket-file assertions elsewhere stay unix-gated for the same
/// reason (named pipes leave no filesystem trace).
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
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
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
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stream", "agent": "claude" }),
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
                    // 只认 agent 内容帧（kind=content）：seed 的 prompt 条目
                    // 在卡片 SEED 阶段就进 transcript——只等非空会在「开轮前」
                    // 放行，忙中提交就会开新轮而不是排队（turn-queue 既有
                    // 探测的边界收窄，fix-pending-queue-liveness 1.1 复用同
                    // 一文件时顺带钉住）。
                    let streamed = v["entries"]
                        .as_array()
                        .is_some_and(|b| b.iter().any(|e| e["kind"].as_str() == Some("content")));
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
    // 用 "stall" 场景（一帧内容后子进程对 ACP 永久沉默、回合不收尾）：
    // WORKING 窗口无限长，close 的丢弃记账不再与回合收尾/排队 drain 竞速
    // （"stream" 的 800ms 窗口是既有的计时边沿，曾在并行负载下偶发把
    // discarded 记成 0）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stall", "agent": "claude" }),
    )
    .await
    .expect("create second session");
    assert_eq!(status, 201, "{body}");
    let key2 = body["key"].as_str().expect("key").to_string();
    let detail2 = format!("{}/api/sessions/{key2}", sb.webui_url());
    wait_for(
        "second session turn to be in flight",
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
                    // 与 submit_turn 的 in-flight 判定同谓词：状态 working
                    // 时提交才确定性入队（prompt-only 会开新轮——seed 的
                    // prompt 条目在卡片 SEED 阶段就进 transcript）。
                    let working = v["status_slug"].as_str() == Some("working");
                    let content_seen = v["entries"]
                        .as_array()
                        .is_some_and(|b| b.iter().any(|e| e["kind"].as_str() == Some("content")));
                    (working && content_seen).then_some(v)
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
#[cfg(target_os = "linux")] // find_child_pid 走 /proc/<pid>/cmdline，非 linux 无从自证
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn core_owned_provider_reaches_router_without_restart() {
    let sb = Sandbox::new("testsuite_e2e", "provider-hotswap");
    // 固定默认端口 8787 会让并行用例互踩——沙箱配置已钉一个 probed 空闲端口。
    let router_port = sb.router_port;

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
                let pid = router_child_pid(watchdog_pid)?;
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
        &format!("{}/api/providers", sb.webui_url()),
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
    // （cmdline 首参校验——进程树定位按 cmdline 子串匹配，先自证锚点）。
    let pid_now = router_child_pid(watchdog_pid);
    assert_eq!(
        pid_now,
        Some(router_pid),
        "router must not restart for a provider change to take effect"
    );
    assert_router_child_cmdline(pid_now);
    assert!(
        watchdog.try_wait().expect("watchdog try_wait").is_none(),
        "watchdog must stay up"
    );
}

/// The router CHILD pid under the watchdog — linux-only `/proc` walk
/// (D1: process-tree assertions are linux-gated, the journey body is not).
#[cfg(target_os = "linux")]
fn router_child_pid(watchdog_pid: u32) -> Option<u32> {
    find_child_pid(watchdog_pid, "router")
}

#[cfg(not(target_os = "linux"))]
fn router_child_pid(_watchdog_pid: u32) -> Option<u32> {
    None
}

/// Anchor check: the matched child really is the router subprocess
/// (argv[1] == "router"). Reads `/proc/{pid}/cmdline` — linux-only, and the
/// `pid_now == Some(router_pid)` equality above is vacuously true on
/// non-linux (`None == None`), so the restart assertion stays portable.
#[cfg(target_os = "linux")]
fn assert_router_child_cmdline(pid_now: Option<u32>) {
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
}

#[cfg(not(target_os = "linux"))]
fn assert_router_child_cmdline(_pid_now: Option<u32>) {}

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
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create default session");
    assert_eq!(status, 201, "create default session: {body}");

    // mode=allow → argv 含 --permission-mode bypassPermissions。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude", "mode": "allow" }),
    )
    .await
    .expect("create allow session");
    assert_eq!(status, 201, "create allow session: {body}");

    // 未知 mode → 400（词汇校验，不静默降级）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "x", "agent": "claude", "mode": "plan" }),
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

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
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

/// session-slash-commands 5.2：命令广告与透传的进程级旅程（Windows 可跑）。
///
/// fake-claude 带 `--advertise-commands`：
/// 1. initialize 握手的命令表经 `AcpEvent::AvailableCommands` → 引擎物化 →
///    会话 detail 的 `available_commands`（goal 带参数提示 + 说明、compact
///    裸命令）——面板的数据源在进程级可见；
/// 2. `/goal some-condition`、`/compact` 两条提交 200 接受后**原样**到达
///    stub（journal in 帧断言原文，回合各收敛到 Done）。
/// 前端「未广告命令拦截」是组件行为（vitest 已覆盖）——后端契约只有一条：
/// 透传无损。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn slash_commands_advertise_and_reach_stub() {
    let sb = Sandbox::new("testsuite_e2e", "slash-commands");
    let journal = sb.advertising_journal_fake_agent();
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 1) 创建 acp（claude 驱动）会话——仓库现状的真实请求形状。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 2) 首回合 Done 且命令表已物化（空表不插键——键在场即非空）。
    let detail = wait_for(
        "session detail to advertise the stub command table",
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
                    let done = v["status_slug"].as_str() == Some("done");
                    let advertised = v["available_commands"]
                        .as_array()
                        .is_some_and(|c| !c.is_empty());
                    (done && advertised).then_some(v)
                })
            }
        },
    )
    .await;
    let commands = detail["available_commands"]
        .as_array()
        .expect("command table");
    let find = |name: &str| {
        commands
            .iter()
            .find(|c| c["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("command {name} must be advertised: {commands:?}"))
    };
    let goal = find("goal");
    assert_eq!(
        goal["hint"], "<condition>",
        "claude argumentHint must map to the panel's param hint"
    );
    assert_eq!(
        goal["description"], "Track a goal across turns",
        "panel description must survive the chain"
    );
    let compact = find("compact");
    assert_eq!(
        compact["description"], "Clear conversation context",
        "compact (universal built-in) must be advertised too"
    );

    // 3) `/goal some-condition`：200 接受 → journal in 帧原文到达 stub →
    //    回合收敛（stub 对未知文本走 hello 场景应答）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "/goal some-condition" }),
    )
    .await
    .expect("send slash command");
    assert_eq!(status, 200, "slash submission must be accepted: {body}");
    wait_for(
        "slash command to reach the stub verbatim (journal in-frame)",
        Duration::from_secs(25),
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
                        .any(|l| {
                            l.contains("\"dir\":\"in\"") && l.contains("\"/goal some-condition\"")
                        })
                        .then_some(())
                })
            }
        },
    )
    .await;
    wait_turn_done(&cli, &sb, &detail_url).await;

    // 4) `/compact`：同款透传（universal built-in 不被任何一侧改写）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "/compact" }),
    )
    .await
    .expect("send /compact");
    assert_eq!(status, 200, "{body}");
    wait_for(
        "/compact to reach the stub verbatim (journal in-frame)",
        Duration::from_secs(25),
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
                        .any(|l| l.contains("\"dir\":\"in\"") && l.contains("\"/compact\""))
                        .then_some(())
                })
            }
        },
    )
    .await;
    wait_turn_done(&cli, &sb, &detail_url).await;
}

/// （workbench-composer-input-polish 2.2/2.3）claude 会话的模型面与切换链路：
/// 内置别名表随快照可达（available_models 含 default/opus/sonnet/haiku），
/// current 从 spawn 缺省被帧观察覆盖为 fake 的真实模型；POST /model 把控制
/// 请求送达 stub（journal 记 model_change），快照乐观跟随，后续回合帧确认
/// 不回跳。全程零真模型调用。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn claude_model_surface_reaches_snapshot_and_switch_round_trips() {
    let sb = Sandbox::new("testsuite_e2e", "claude-model-surface");
    let journal = sb.journal_fake_agent();
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 1) 首回合 Done 后：别名表随快照可达，current 已被 init 帧观察覆盖
    //    （fake 报 "fake"）——快照拼装与覆盖次序的进程级证据。
    let detail = wait_for(
        "claude model surface to reach the snapshot (alias table + observed current)",
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
                    let done = v["status_slug"].as_str() == Some("done");
                    let models = v["available_models"].as_array()?;
                    let aliased = ["default", "opus", "sonnet", "haiku"]
                        .iter()
                        .all(|a| models.iter().any(|m| m.as_str() == Some(*a)));
                    let observed = v["current_model"].as_str() == Some("fake");
                    (done && aliased && observed).then_some(v)
                })
            }
        },
    )
    .await;
    assert_eq!(
        detail["current_model"], "fake",
        "frame observation must overwrite the spawn-time default: {detail}"
    );

    // 2) 切换到 "opus"：webui 只投递（200）→ 驱动 set_model 控制请求 →
    //    乐观 ModelChanged → 快照 current 同步。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/model", sb.webui_url()),
        serde_json::json!({ "model_id": "opus" }),
    )
    .await
    .expect("switch model");
    assert_eq!(status, 200, "model switch must be delivered: {body}");
    wait_for(
        "current_model to follow the optimistic switch",
        Duration::from_secs(15),
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
                    (v["current_model"].as_str() == Some("opus")).then_some(v)
                })
            }
        },
    )
    .await;
    wait_for(
        "set_model control request to reach the stub (journal model_change)",
        Duration::from_secs(15),
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
                        .any(|l| l.contains("model_change") && l.contains("\"model\":\"opus\""))
                        .then_some(())
                })
            }
        },
    )
    .await;

    // 3) 下一回合：assistant 帧报告已切换的模型（fake 行为）→ 观察值与
    //    乐观值一致，current 不回跳（帧确认半边）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("follow-up message");
    assert_eq!(status, 200, "{body}");
    wait_turn_done(&cli, &sb, &detail_url).await;
    let after = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the confirming turn")
        .1;
    assert_eq!(
        after["current_model"], "opus",
        "confirmed switch must survive the confirming frames: {after}"
    );
    assert_eq!(
        after["available_models"]
            .as_array()
            .expect("model list survives")
            .len(),
        4
    );
}

/// Poll the session detail until the current turn settles to Done.
async fn wait_turn_done(cli: &reqwest::Client, sb: &Sandbox, detail_url: &str) {
    wait_for(
        "session turn to reach Done",
        Duration::from_secs(25),
        &sb.path.clone(),
        {
            let cli = cli.clone();
            let url = detail_url.to_string();
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
                    (v["status_slug"].as_str() == Some("done")).then_some(())
                })
            }
        },
    )
    .await;
}

/// add-workspace-root 4.3：workspace root 执法的进程级旅程——「注册后收紧根」
/// 变体，Windows 可跑（只重启 webui 单进程，不依赖 unix 信号）。
///
/// webui 以 env 携带更窄的根重启（`SEBAS_WORKSPACE_ROOT` > `[workspace] root`），
/// core 与其上的会话原地不动：注册过的项目就此成为「越界历史项目」。覆盖：
/// 1) 注册越界 → 400「路径超出允许范围」，越界 + 不存在同文案（范围判定
///    先行于存在性，不借文案差异探测根外目录）；
/// 2) 越界历史项目从 GET /api/projects 隐藏，branch 探测按不可达；
/// 3) 绑定它的会话 detail / message / switch 400 拒绝，archive / close 放行
///    （围栏不锁垃圾）；
/// 4) 全程 GET /api/summary 健康（沙箱没有被执法误伤）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn workspace_root_enforcement_after_tightening() {
    let mut sb = Sandbox::new("testsuite_e2e", "workspace-root");
    let cli = http_client();
    let _core = sb.spawn_core();
    let mut webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 界内注册（沙箱 work 目录）+ 两个绑定它的 0-turn 占位会话。占位形态
    // 足够：执法看的是快照上的 project_dir，与有没有跑过 turn 无关。
    let work_dir = sb.path.join("work");
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({ "path": support::forward_slash(&work_dir) }),
    )
    .await
    .expect("register in-scope project");
    assert_eq!(status, 201, "in-scope register: {body}");
    let project_id = body["id"].as_str().expect("project id").to_string();

    let key_a = create_project_placeholder(&cli, &sb, &project_id, "a").await;
    let key_b = create_project_placeholder(&cli, &sb, &project_id, "b").await;

    // 收紧前 detail 可读——证明下面的 400 来自收紧，不是占位形态本身。
    let (status, detail_before) =
        get_json_status(&cli, &format!("{}/api/sessions/{key_a}", sb.webui_url()))
            .await
            .expect("detail before tightening");
    assert_eq!(
        status, 200,
        "detail must read fine while in scope: {detail_before}"
    );

    // 收紧根：只重启 webui（新端口避免复绑等待），core 与会话不动。
    webui.kill().await.expect("kill webui");
    let scope = sb.path.join("scope");
    std::fs::create_dir_all(&scope).expect("mkdir scope");
    let new_port = free_port();
    let config = std::fs::read_to_string(&sb.config_path).expect("read config");
    let patched = config.replace(
        &format!("port = {}", sb.webui_port),
        &format!("port = {new_port}"),
    );
    assert_ne!(patched, config, "[watchdog.webui] port line not found");
    std::fs::write(&sb.config_path, patched).expect("write config");
    sb.webui_port = new_port;
    let scope_s = support::forward_slash(&scope);
    let _webui2 = sb.spawn(
        &["webui", "-c", &support::forward_slash(&sb.config_path)],
        &sb.core_secret,
        &[("SEBAS_WORKSPACE_ROOT", &scope_s)],
        &sb.webui_log,
    );
    wait_reachable(&cli, &sb).await;

    // 1) 越界注册 400：界内已存在的 work 与「越界 + 不存在」的幽灵同文案。
    let (status, oob) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({ "path": support::forward_slash(&work_dir) }),
    )
    .await
    .expect("register out-of-scope existing");
    assert_eq!(status, 400, "out-of-scope register must 400: {oob}");
    let oob_msg = oob["error"].as_str().expect("error text").to_string();
    assert!(
        oob_msg.contains("路径超出允许范围"),
        "unexpected rejection text: {oob_msg}"
    );
    let ghost = sb.path.join("ghost-never-created");
    let (status, ghost_body) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({ "path": support::forward_slash(&ghost) }),
    )
    .await
    .expect("register out-of-scope nonexistent");
    assert_eq!(
        status, 400,
        "nonexistent out-of-scope must 400: {ghost_body}"
    );
    assert_eq!(
        ghost_body["error"], oob["error"],
        "越界 + 不存在必须同文案（范围先行于存在性）: {ghost_body} vs {oob}"
    );

    // 对照：界内注册照常成功——400 是执法，不是项目面坏了。
    let inside = scope.join("inside");
    std::fs::create_dir_all(&inside).expect("mkdir inside");
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/projects", sb.webui_url()),
        serde_json::json!({ "path": support::forward_slash(&inside) }),
    )
    .await
    .expect("register in-scope control");
    assert_eq!(status, 201, "in-scope register after tightening: {body}");
    let inside_id = body["id"].as_str().expect("control project id").to_string();

    // 2) 越界历史项目从列表隐藏；branch 探测按不可达。
    let projects = cli
        .get(format!("{}/api/projects", sb.webui_url()))
        .send()
        .await
        .expect("list projects")
        .json::<serde_json::Value>()
        .await
        .expect("projects json");
    let listed = projects.to_string();
    assert!(
        !listed.contains(&project_id),
        "越界历史项目必须从列表隐藏: {projects}"
    );
    assert!(
        listed.contains(&inside_id),
        "界内对照项目必须在列: {projects}"
    );
    let branch = cli
        .get(format!(
            "{}/api/projects/{project_id}/branch",
            sb.webui_url()
        ))
        .send()
        .await
        .expect("branch probe")
        .json::<serde_json::Value>()
        .await
        .expect("branch json");
    assert_eq!(
        branch["accessible"], false,
        "越界项目的 branch 探测必须按不可达: {branch}"
    );
    assert!(
        branch["branch"].is_null(),
        "越界项目不得泄漏分支名: {branch}"
    );

    // 3) 会话面：detail / message / switch 拒绝（同一文案），archive /
    //    close 放行（围栏不锁垃圾）。
    let out_of_scope = |what: &str, body: serde_json::Value| {
        assert!(
            body["error"]
                .as_str()
                .unwrap_or_default()
                .starts_with("项目目录超出允许范围"),
            "{what} must carry the typed rejection: {body}"
        );
    };
    let (status, body) = get_json_status(&cli, &format!("{}/api/sessions/{key_a}", sb.webui_url()))
        .await
        .expect("detail out-of-scope");
    assert_eq!(
        status, 400,
        "detail on out-of-scope session must 400: {body}"
    );
    out_of_scope("detail", body);
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key_a}/message", sb.webui_url()),
        serde_json::json!({ "message": "hello" }),
    )
    .await
    .expect("message out-of-scope");
    assert_eq!(
        status, 400,
        "message on out-of-scope session must 400: {body}"
    );
    out_of_scope("message", body);
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key_a}/switch", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("switch out-of-scope");
    assert_eq!(
        status, 400,
        "switch on out-of-scope session must 400: {body}"
    );
    out_of_scope("switch", body);
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key_a}/archive", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("archive out-of-scope session");
    assert_eq!(status, 200, "archive must pass the fence: {body}");
    assert_eq!(body["status"], "archived", "{body}");
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key_b}/close", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("close out-of-scope session");
    assert_eq!(status, 200, "close must pass the fence: {body}");
    assert_eq!(body["status"], "closed", "{body}");

    // 4) 全程健康：执法没有伤到沙箱本身的可用面。
    let summary = cli
        .get(format!("{}/api/summary", sb.webui_url()))
        .send()
        .await
        .expect("summary")
        .json::<serde_json::Value>()
        .await
        .expect("summary json");
    assert_eq!(
        summary["reachability"]["ok"], true,
        "sandbox must stay healthy: {summary}"
    );
}

/// POST a 0-turn placeholder session bound to `project_id`; returns the
/// encoded session key.
async fn create_project_placeholder(
    cli: &reqwest::Client,
    sb: &Sandbox,
    project_id: &str,
    tag: &str,
) -> String {
    let (status, body) = post_json(
        cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "agent": "claude", "project_id": project_id }),
    )
    .await
    .unwrap_or_else(|e| panic!("create placeholder {tag}: {e}"));
    assert_eq!(status, 201, "placeholder {tag}: {body}");
    body["key"].as_str().expect("key").to_string()
}

/// GET a URL, return (status, json body). Err on transport failure.
async fn get_json_status(
    cli: &reqwest::Client,
    url: &str,
) -> Result<(u16, serde_json::Value), String> {
    let resp = cli
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    let status = resp.status().as_u16();
    let json = resp
        .json()
        .await
        .map_err(|e| format!("body of {url}: {e}"))?;
    Ok((status, json))
}

/// workbench-live-conversation-flow 3.1/7.1：聚焦即拉起的进程级旅程。
///
/// 0-turn 占位（无 prompt）→ `POST /api/sessions/{key}/activate` 一次 =
/// `started`（无 prompt 拉起子进程，占位离开 starting 态且没有任何回合）
/// → 再 activate = `already-running`（幂等，无重复 spawn）。模型芯片的
/// 数据源（fakeacp 的 configOptions）由沙箱冒烟覆盖——专用 claude 驱动
/// 不报 configOptions，这里不断言模型表。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn activate_placeholder_spawns_without_prompt() {
    let sb = Sandbox::new("testsuite_e2e", "activate-placeholder");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 0-turn 占位：创建请求不带 prompt。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "agent": "claude" }),
    )
    .await
    .expect("create placeholder");
    assert_eq!(status, 201, "create placeholder: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let activate_url = format!("{}/api/sessions/{key}/activate", sb.webui_url());

    // 第一次激活 = started（无 prompt 拉起）。
    let (status, body) = post_json(&cli, &activate_url, serde_json::json!({}))
        .await
        .expect("activate #1");
    assert_eq!(status, 200, "activate #1: {body}");
    assert_eq!(body["status"], "started", "activate #1: {body}");

    // 子进程拉起后离开 starting 态（无 prompt、无回合——0-turn 占位保持）。
    let detail = wait_for(
        "placeholder to leave starting after activate",
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
                    (v["status_slug"].as_str() != Some("starting")).then_some(v)
                })
            }
        },
    )
    .await;
    assert_eq!(
        detail["entries"].as_array().map(Vec::len),
        Some(0),
        "activation must not run a turn: {detail}"
    );

    // 幂等：第二次激活 = already-running（无第二次 spawn）。
    let (status, body) = post_json(&cli, &activate_url, serde_json::json!({}))
        .await
        .expect("activate #2");
    assert_eq!(status, 200, "activate #2: {body}");
    assert_eq!(body["status"], "already-running", "activate #2: {body}");
}

/// fix-webui-qa-defects 3.3（session-lifecycle「idle placeholder is never
/// stall-settled」的进程级回归）：0-turn 占位经聚焦拉起（activate：无 prompt
/// spawn、握手 lazy seed 出 SEED 卡、transcript 恒空——QA 幽灵回合的实体）
/// 后闲置超过配置调小的 `turn_stall_timeout`（3s）——
/// 1) detail 无任何 error 条目：占位不构成在飞回合，看门狗绝不注入合成
///    「回合停滞被强制收尾」；
/// 2) 状态可写：首条消息照常开轮并完成完整回合；完成后再次越过阈值，
///    transcript 依旧无合成错误（看门狗不误伤已收回合）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn idle_placeholder_never_stall_settles_and_stays_writable() {
    let sb = Sandbox::new("testsuite_e2e", "placeholder-idle-stall");
    sb.set_turn_stall_timeout(3);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 0-turn 占位（创建请求不带 prompt）→ 聚焦拉起（activate #1 = started）。
    let project_id = scene_project_id(&cli, &sb).await;
    let key = create_project_placeholder(&cli, &sb, &project_id, "idle-stall").await;
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let activate_url = format!("{}/api/sessions/{key}/activate", sb.webui_url());
    let (status, body) = post_json(&cli, &activate_url, serde_json::json!({}))
        .await
        .expect("activate placeholder");
    assert_eq!(status, 200, "activate placeholder: {body}");
    assert_eq!(body["status"], "started", "activate placeholder: {body}");

    // 子进程拉起后离开 starting 态（0-turn 占位：无 prompt、零条目）。
    let hint = sb.path.clone();
    wait_for(
        "placeholder to leave starting after activate",
        Duration::from_secs(25),
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
                    (v["status_slug"].as_str() != Some("starting")).then_some(v)
                })
            }
        },
    )
    .await;

    // 闲置超过阈值（3s 配置，8s 观察窗）：detail 保持干净——零 error 条目、
    // 无「回合停滞」合成文案（占位幽灵回合的进程级反证）。
    tokio::time::sleep(Duration::from_secs(8)).await;
    let (status, v) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the idle window");
    assert_eq!(status, 200, "detail after the idle window: {v}");
    let entries = v["entries"].as_array().cloned().unwrap_or_default();
    assert!(
        entries
            .iter()
            .all(|e| e["element_type"].as_str() != Some("error")),
        "an idle 0-turn placeholder must never carry a synthetic error entry: {entries:?}"
    );

    // 状态可写：首条消息照常开轮并完成完整回合（占位豁免绝不外溢到真实
    // 回合——看门狗对它照常计时，回合正常完成即不受影响）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "first real message" }),
    )
    .await
    .expect("message the idle placeholder");
    assert_eq!(status, 200, "the idle placeholder must stay writable: {body}");
    wait_turn_done(&cli, &sb, &detail_url).await;

    // 完成后再次越过阈值：看门狗不误伤已收回合，transcript 仍无合成错误，
    // 且真实回合的事实（操作者 prompt + fake-claude 回复）完整在场。
    tokio::time::sleep(Duration::from_secs(6)).await;
    let (_status, v) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the post-done window");
    let entries = v["entries"].as_array().cloned().unwrap_or_default();
    assert!(
        entries
            .iter()
            .all(|e| e["element_type"].as_str() != Some("error")),
        "a completed turn must not gain a synthetic error after the timeout: {entries:?}"
    );
    let transcript = entries
        .iter()
        .filter_map(|e| e["content"].as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        transcript.contains("first real message")
            && transcript.contains("hello")
            && transcript.contains("world"),
        "the real turn's prompt and reply must be intact: {transcript:?}"
    );
}

/// workbench-live-conversation-flow 2.1/7.1：回合内容实时流式的进程级
/// 旅程——订阅 `/ws` 后提交一条消息，`turn.append` 帧（合并窗批帧，条目
/// position 单调）必须在回合结束前/后到达，且内容覆盖操作员 prompt 与
/// agent 回复。快照重取收敛由组件测试覆盖，这里只钉传输契约。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn turn_appends_stream_over_ws() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let sb = Sandbox::new("testsuite_e2e", "turn-append-ws");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 创建并等首回合收敛（fake-claude 对任意 prompt 回 "hello world"）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stream me", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();

    // 先订阅（快照帧先行），再提交第二条消息——帧在提交之后到达证明是
    // 推送而非轮询假象。
    let ws_url = sb.webui_url().replace("http://", "ws://") + "/ws";
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("ws connect");
    // 排干订阅快照帧。
    let _ = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "second turn please" }),
    )
    .await
    .expect("send message");
    assert_eq!(status, 200, "send: {body}");

    // 收 8 秒帧：必须出现 turn.append，且条目含 prompt 与 agent 内容。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut prompt_seen = false;
    let mut agent_seen = false;
    while tokio::time::Instant::now() < deadline {
        let frame = match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws read failed: {e}"),
            Ok(None) => panic!("ws closed early"),
            Err(_) => break, // 2s 无帧：回合内容已全部到齐（或超时判负）
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&frame) else {
            continue;
        };
        if v["method"] != "turn.append" {
            continue;
        }
        let entries = v["params"]["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for e in &entries {
            let kind = e["kind"].as_str().unwrap_or("");
            let content = e["content"].as_str().unwrap_or("");
            if kind == "prompt" && content.contains("second turn please") {
                prompt_seen = true;
            }
            if kind == "content" && !content.is_empty() && e["position"].as_u64().unwrap_or(0) > 1 {
                agent_seen = true;
            }
        }
        if prompt_seen && agent_seen {
            break;
        }
    }
    assert!(prompt_seen, "prompt entry must stream over ws");
    assert!(agent_seen, "agent content must stream over ws");
    let _ = ws.close(None).await;
}

/// fix-pending-queue-liveness 1.1/4.1：停滞看门狗自愈。
///
/// fake-claude 的 `stall` 场景：首帧内容让卡片 FSM 进 WORKING（回合确实在
/// 跑），随后子进程**活着但对 ACP 完全沉默**（liveness 探测照常应答——驱动
/// 的 hang 升级链因此不触发）。这是「队列前进 100% 依赖终态事件」的结构性
/// 缺陷的进程级复现：忙中提交入队后，旧引擎永不 drain。
///
/// 修复后：`[dispatch] turn_stall_timeout = 2` 的看门狗在阈值后强制收尾
/// 停滞回合（SEED/WORKING → DONE）、drain 队头，队列自愈前进；core.log 留
/// 有点名会话与释放条目数的 warn（TurnStalled 通知事实，前端据此弹分级
/// 通知——呈现链路由 webui api/ws 单测钉住）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn stalled_turn_force_settles_and_the_queue_self_heals() {
    let sb = Sandbox::new("testsuite_e2e", "turn-stall");
    sb.set_turn_stall_timeout(2);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 创建会话（"stall" → 一帧内容后子进程沉默）→ 等首帧内容上屏。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "stall", "agent": "claude" }),
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

    // 忙中提交 → 入队；且引擎事实 turn_engaged=true 随 payload 下发
    // （提交控件的排队/停止形态数据源）。
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "queued while stalled" }),
    )
    .await
    .expect("submit while stalled");
    assert_eq!(status, 200, "busy-time submission must be accepted: {resp}");
    let mid: serde_json::Value = cli
        .get(&detail_url)
        .send()
        .await
        .expect("detail mid-stall")
        .json()
        .await
        .expect("detail json");
    assert_eq!(
        mid["turn_engaged"], true,
        "a stalled-but-engaged turn must report turn_engaged: {mid}"
    );
    assert_eq!(
        mid["pending"].as_array().map(|p| p.len()),
        Some(1),
        "the submission rides in pending behind the stalled turn: {mid}"
    );

    // 看门狗收尾 + 队列自愈：queued 提交开轮（prompt 进 transcript、
    // pending 清空），transcript 就地点名收尾原因。
    wait_for(
        "watchdog to settle the stalled turn and start the queued one",
        Duration::from_secs(45),
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
                    let drained = v["pending"].as_array().map(|p| p.is_empty()) == Some(true);
                    let queued_started = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| {
                            e["kind"].as_str() == Some("prompt")
                                && e["content"].as_str() == Some("queued while stalled")
                        })
                    });
                    let explained = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| {
                            e["element_type"].as_str() == Some("error")
                                && e["content"]
                                    .as_str()
                                    .is_some_and(|c| c.contains("回合停滞被强制收尾"))
                        })
                    });
                    (drained && queued_started && explained).then_some(v)
                })
            }
        },
    )
    .await;

    // 通知事实可达：core.log 的 warn 点名会话与释放条目数（TurnStalled
    // 事件的发射点）。
    wait_for(
        "core.log to carry the stall warning",
        Duration::from_secs(10),
        &hint,
        {
            let log = sb.core_log.clone();
            move || {
                let log = log.clone();
                Box::pin(async move {
                    let text = std::fs::read_to_string(&log).ok()?;
                    text.contains("turn stalled").then_some(())
                })
            }
        },
    )
    .await;
}

/// fix-pending-queue-liveness 1.1（泊车豁免半边）：泊在权限请求上的回合
/// **不计时**——超过 `turn_stall_timeout` 后看门狗不得收尾，排队提交保持
/// 入栈（等的是操作者的批复，不是引擎的假设）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn parked_permission_does_not_trip_the_stall_guard() {
    let sb = Sandbox::new("testsuite_e2e", "turn-stall-parked");
    sb.set_turn_stall_timeout(2);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // "perm" → Bash 工具调用被 PreToolUse 审批泊住（等 hook 决定）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "perm", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();

    // 忙中提交（泊车中 turn 在飞）→ 入队。
    wait_for(
        "permission prompt to park the turn (tool entry visible)",
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
                    let tool_seen = v["entries"].as_array().is_some_and(|entries| {
                        entries
                            .iter()
                            .any(|e| e["element_type"].as_str() == Some("tool"))
                    });
                    tool_seen.then_some(v)
                })
            }
        },
    )
    .await;
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "queued behind the permission" }),
    )
    .await
    .expect("submit while parked");
    assert_eq!(
        status, 200,
        "parked-time submission must be accepted: {resp}"
    );

    // 静置越过阈值（2s ×3 + 扫描间隔余量）：看门狗不得收尾。
    tokio::time::sleep(Duration::from_secs(9)).await;
    let after: serde_json::Value = cli
        .get(&detail_url)
        .send()
        .await
        .expect("detail after the window")
        .json()
        .await
        .expect("detail json");
    assert_eq!(
        after["pending"].as_array().map(|p| p.len()),
        Some(1),
        "the guard must not fire while a permission is parked: {after}"
    );
    assert!(
        !after["status_slug"].as_str().is_some_and(|s| s == "done"),
        "the parked turn must stay in flight: {after}"
    );
    let log = std::fs::read_to_string(&sb.core_log).unwrap_or_default();
    assert!(
        !log.contains("turn stalled"),
        "no stall warning may be logged for a parked session"
    );
}

// ── feishu-free permission approval loop (the im surface over the core channel) ──
//
// 飞书在权限回路里的唯一职责是「权限卡片呈现 + card.action.trigger 回调」；
// 卡片之前的整条审批回路（agent 泊车 → PermissionNotice 广播 → 订阅流
// ApprovalRequested 帧 → ApprovalAnswer 回灌 → 泊住的 hook 复活 → 回合完成）
// 与飞书零耦合，用 fake-claude 的 "perm" 场景（真实 hook_callback 泊车，
// allow/deny 决定直接改写 tool_result）经核心通道裸帧走全环。这一组用例
// 就是那条「长链路的前 9 步」：任何一截断裂（驱动泊车、广播、帧下发、
// 应答路由、oneshot 复活）都会在这里当场爆。卡片回调之后的解析/路由语义
// 由 sebas-feishu event_parse_test 与 sebas-im frontend 单测覆盖。

use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use support::forward_slash;

use sebas::core_channel::protocol::{
    ChannelHandshake, CoreChannelRequest, CoreChannelResponse, SessionStreamFrame,
};
use sebas_channels::ChannelKey;
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice};

/// 把沙箱 config 的 `channel_path` 从相对名补成绝对路径：Sandbox 写的相对名
/// 依赖「子进程 cwd=沙箱」的字符串映射（Windows named pipe 名从路径字符串
/// 确定性派生），测试进程直连时必须与服务端同一字符串——绝对路径对两端都
/// 成立，也顺带满足 Unix 上的文件语义。只影响本组用例自己的沙箱实例。
fn pin_absolute_channel(sb: &Sandbox) {
    let cfg = std::fs::read_to_string(&sb.config_path).expect("read sandbox config");
    let patched = cfg.replace(
        "channel_path = \"core-channel.sock\"",
        &format!("channel_path = \"{}\"", forward_slash(&sb.channel_path)),
    );
    assert_ne!(cfg, patched, "channel_path line not found in sandbox config");
    std::fs::write(&sb.config_path, patched).expect("write sandbox config");
}

/// 通道可连接性轮询（named pipe 无文件残留，不能靠 path.exists()）。
async fn wait_channel_accept(sb: &Sandbox) {
    let hint = sb.path.clone();
    wait_for("core channel to accept a handshake", Duration::from_secs(20), &hint, move || {
        let path = sb.channel_path.clone();
        let secret = sb.core_secret.clone();
        Box::pin(async move {
            let Ok(stream) = sebas_ipc::connect(&path).await else {
                return None;
            };
            let (r, mut w) = sebas_ipc::split(stream);
            let hs = serde_json::to_string(&ChannelHandshake { secret }).unwrap();
            if w.write_all(hs.as_bytes()).await.is_err()
                || w.write_all(b"\n").await.is_err()
                || w.flush().await.is_err()
            {
                return None;
            }
            let mut reader = BufReader::new(r);
            let mut ack = String::new();
            match tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut ack)).await {
                Ok(Ok(1..)) => Some(()),
                _ => None,
            }
        })
    })
    .await;
}
/// 开一条订阅连接：握手 + Subscribe，吃掉 ack 与 Snapshot 帧，返回读端。
async fn open_subscriber(sb: &Sandbox) -> BufReader<sebas_ipc::ReadHalf> {
    let stream = sebas_ipc::connect(&sb.channel_path).await.expect("subscriber connect");
    let (r, mut w) = sebas_ipc::split(stream);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: sb.core_secret.clone(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.write_all(serde_json::to_string(&CoreChannelRequest::Subscribe).unwrap().as_bytes())
        .await
        .unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();
    let mut reader = BufReader::new(r);
    let mut ack = String::new();
    reader.read_line(&mut ack).await.expect("handshake ack");
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("snapshot frame");
    let frame: SessionStreamFrame = serde_json::from_str(line.trim()).expect("frame json");
    assert!(
        matches!(frame, SessionStreamFrame::Snapshot { .. }),
        "first stream frame must be the snapshot: {line}"
    );
    reader
}

/// 单发请求连接（订阅连接只推流不处理请求）：握手 → 请求 → 读响应。
async fn raw_channel_request(
    channel: &Path,
    secret: &str,
    req: &CoreChannelRequest,
) -> CoreChannelResponse {
    let stream = sebas_ipc::connect(channel).await.expect("request connect");
    let (r, mut w) = sebas_ipc::split(stream);
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: secret.to_string(),
    })
    .unwrap();
    w.write_all(hs.as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.write_all(serde_json::to_string(req).unwrap().as_bytes()).await.unwrap();
    w.write_all(b"\n").await.unwrap();
    w.flush().await.unwrap();
    let mut reader = BufReader::new(r);
    let mut ack = String::new();
    reader.read_line(&mut ack).await.expect("handshake ack");
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line))
        .await
        .expect("response in time")
        .expect("response line");
    serde_json::from_str(line.trim()).expect("response json")
}

/// 从订阅流读一帧（超时由调用方的 tick 循环套）。
async fn read_frame(reader: &mut BufReader<sebas_ipc::ReadHalf>) -> SessionStreamFrame {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("stream alive");
    serde_json::from_str(line.trim()).expect("frame json")
}

/// 等 Turns 里出现第 `n` 个含 `marker` 的条目（transcript 是唯一事实）。
async fn wait_turn_marker(sb: &Sandbox, key: &ChannelKey, marker: &str, n: usize) {
    let hint = sb.path.clone();
    let path = sb.channel_path.clone();
    let secret = sb.core_secret.clone();
    let key = key.clone();
    let marker = marker.to_string();
    wait_for(
        &format!("{n}-th transcript marker `{marker}`"),
        Duration::from_secs(30),
        &hint,
        move || {
            let path = path.clone();
            let secret = secret.clone();
            let key = key.clone();
            let marker = marker.clone();
            Box::pin(async move {
                match raw_channel_request(
                    &path,
                    &secret,
                    &CoreChannelRequest::Turns { key, from: 0 },
                )
                .await
                {
                    CoreChannelResponse::Turns { entries } => {
                        let count = entries
                            .iter()
                            .filter(|e| e.content.contains(&marker))
                            .count();
                        (count >= n).then_some(())
                    }
                    other => panic!("Turns must return Turns, got {other:?}"),
                }
            })
        },
    )
    .await;
}

/// 全环共享旅程：EnsureMessage("perm") → 真实泊车 → 订阅流收到
/// ApprovalRequested → ApprovalAnswer{decision} → transcript 出现 `marker`。
/// 返回收到的 notice 供用例做字段断言。
async fn drive_parked_perm_to_decision(
    sb: &Sandbox,
    mut reader: BufReader<sebas_ipc::ReadHalf>,
    decision: PermissionDecision,
    marker: &str,
) -> PermissionNotice {
    let key = ChannelKey::feishu("oc_perm_loop", None);
    let resp = raw_channel_request(
        &sb.channel_path,
        &sb.core_secret,
        &CoreChannelRequest::EnsureMessage {
            key: key.clone(),
            message: "perm".into(),
            attachments: vec![],
        },
    )
    .await;
    assert!(
        matches!(resp, CoreChannelResponse::Ok | CoreChannelResponse::Spawned { .. }),
        "ensure must be accepted: {resp:?}"
    );

    // 真实泊车产生的 ApprovalRequested（非合成事件）：60s 预算内逐帧吃，
    // 中间的会话生命周期 Event 帧全部越过。
    let notice = {
        let wait = async {
            loop {
                if let SessionStreamFrame::ApprovalRequested { notice } =
                    read_frame(&mut reader).await
                {
                    break notice;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(60), wait)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "ApprovalRequested frame did not arrive within 60s; logs at {}",
                    sb.path.display()
                )
            })
    };
    assert_eq!(notice.tool_name, "Bash", "parked tool: {notice:?}");
    assert_eq!(
        notice.args["command"].as_str(),
        Some("rm -rf /"),
        "parked tool args: {notice:?}"
    );

    // 应答走独立请求连接 → Ok。
    let resp = raw_channel_request(
        &sb.channel_path,
        &sb.core_secret,
        &CoreChannelRequest::ApprovalAnswer {
            request_id: notice.request_id.clone(),
            decision,
        },
    )
    .await;
    assert!(
        matches!(resp, CoreChannelResponse::Ok),
        "answer must be accepted: {resp:?}"
    );

    // 泊住的 hook 复活：transcript 出现决定对应的 tool_result。
    wait_turn_marker(sb, &key, marker, 1).await;
    notice
}

/// allow_once：只放行本次，回合完成且 transcript 记录执行痕迹。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn permission_loop_allow_once_over_core_channel() {
    let sb = Sandbox::new("testsuite_e2e", "perm-loop-allow");
    pin_absolute_channel(&sb);
    let _core = sb.spawn_core();
    wait_channel_accept(&sb).await;
    let reader = open_subscriber(&sb).await;
    drive_parked_perm_to_decision(&sb, reader, PermissionDecision::AllowOnce, "perm done").await;
}

/// deny：回合完成且 transcript 记录拒绝语义——fail-closed 的应答半边。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn permission_loop_deny_over_core_channel() {
    let sb = Sandbox::new("testsuite_e2e", "perm-loop-deny");
    pin_absolute_channel(&sb);
    let _core = sb.spawn_core();
    wait_channel_accept(&sb).await;
    let reader = open_subscriber(&sb).await;
    drive_parked_perm_to_decision(&sb, reader, PermissionDecision::Deny, "denied by fake").await;
}

/// allow_session：放行本次 + 会话切 auto——同 key 的第二个 "perm" 回合
/// **不再泊车**（无新 ApprovalRequested），直接执行完成。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn permission_loop_allow_session_switches_auto_over_core_channel() {
    let sb = Sandbox::new("testsuite_e2e", "perm-loop-session");
    pin_absolute_channel(&sb);
    let _core = sb.spawn_core();
    wait_channel_accept(&sb).await;
    let reader = open_subscriber(&sb).await;
    let key = ChannelKey::feishu("oc_perm_loop", None);
    drive_parked_perm_to_decision(&sb, reader, PermissionDecision::AllowSession, "perm done").await;

    // 第二回合：mode 已 auto → fake-claude 跳过 hook 直接执行。泊住的回合
    // 永远写不出第二个 tool_result，所以「第二个 marker 到账」同时就是
    // 「未再泊车」的对账——mode 没切上时这里 30s 超时爆掉。
    let resp = raw_channel_request(
        &sb.channel_path,
        &sb.core_secret,
        &CoreChannelRequest::EnsureMessage {
            key: key.clone(),
            message: "perm".into(),
            attachments: vec![],
        },
    )
    .await;
    assert!(
        matches!(resp, CoreChannelResponse::Ok),
        "second ensure must be accepted: {resp:?}"
    );
    wait_turn_marker(&sb, &key, "perm done", 2).await;
}

// ── fix-webui-streaming-liveness 6.1/6.2/6.3：流式活性与瘦 summary ─────────

/// One `turn.append` notification's **content** entry texts (prompt/tool
/// entries excluded) — the per-frame streaming unit the assertions count.
fn append_frame_texts(ev: &serde_json::Value) -> Vec<String> {
    ev["params"]["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["kind"].as_str() == Some("content"))
                .filter_map(|e| e["content"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// fix-webui-streaming-liveness 6.1：claude 会话流式 e2e（回合进行中分多次帧
/// 到达，非一次整块）。fake-claude "drip" 场景以 400ms 间隔发 3 个
/// partial-stream 文本块——每个都落在独立的 core-channel 合并窗（250ms）里，
/// webui WS 因此收到**多帧**；首帧到达时刻的 HTTP 快照证明内容在回合内可见
/// （状态仍 working），最终 transcript 与帧序列对账一致且**逐字各一次**——
/// partial 流 + 完整帧不产生重复（driver 1.3 的进程级证据）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn claude_turn_streams_multiple_frames_to_the_webui() {
    let sb = Sandbox::new("testsuite_e2e", "claude-drip");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 先连 WS，再建会话：订阅先于一切落账，首帧起不漏。webui 启动会与
    // core 的 socket bind 竞速——首个流式订阅尝试可能失败、经 1s 退避后才
    // 连上；`session.resync` 随每次订阅快照到达，等到它（或短超时内没等到
    // = 订阅已在前置检查期间上线，不会再来）再创建会话。
    let mut ws: WsStream = ws_connect(&sb.webui_url()).await;
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let ev = next_ws_frame(&mut ws).await;
            if ev["method"] == "session.resync" {
                break;
            }
        }
    })
    .await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "drip", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 回合收尾由 HTTP 轮询判定（claude kind 收尾无 transcript 标记帧）。
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    let poll_cli = cli.clone();
    let poll_url = detail_url.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(v) = poll_cli.get(&poll_url).send().await
                && let Ok(j) = v.json::<serde_json::Value>().await
                && j["status_slug"].as_str() == Some("done")
            {
                let _ = done_tx.send(j);
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // 读帧：记录每个 turn.append 的 content 帧内容；首帧到达即抓一份 HTTP
    // 快照（回合必须仍在飞、transcript 已有首块）。
    let mut frames: Vec<Vec<String>> = Vec::new();
    let mut mid_turn: Option<serde_json::Value> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                panic!("timed out waiting for the drip turn; frames so far: {frames:?}")
            }
            done = &mut done_rx => {
                let done = done.expect("done watcher alive");
                assert_eq!(
                    done["status_slug"].as_str(),
                    Some("done"),
                    "turn must end Done: {done}"
                );
                break;
            }
            ev = next_ws_frame(&mut ws) => {
                let ev = ev;
                if ev["method"] == "turn.append"
                    && ev["params"]["session_id"] == key.as_str()
                {
                    let texts = append_frame_texts(&ev);
                    if !texts.is_empty() {
                        if mid_turn.is_none() {
                            let snap: serde_json::Value = cli
                                .get(&detail_url)
                                .send()
                                .await
                                .expect("mid-turn detail")
                                .json()
                                .await
                                .expect("mid-turn json");
                            assert_eq!(
                                snap["status_slug"].as_str(), Some("working"),
                                "the first streamed frame must arrive while the turn is in flight: {snap}"
                            );
                            let streamed: String = snap["entries"]
                                .as_array()
                                .map(|b| {
                                    b.iter()
                                        .filter(|e| e["kind"].as_str() == Some("content"))
                                        .filter_map(|e| e["content"].as_str())
                                        .collect::<Vec<_>>()
                                        .join("")
                                })
                                .unwrap_or_default();
                            assert!(
                                streamed.contains("drip0"),
                                "the first chunk must already be visible mid-turn: {streamed:?}"
                            );
                            mid_turn = Some(snap);
                        }
                        frames.push(texts);
                    }
                }
            }
        }
    }

    // 收尾竞速窗口：最后一个 content 帧可能与 done 信号同时就绪（250ms 合
    // 并窗的冲刷略迟于状态翻转），短暂排水补齐「逐字各一次」的对账——多帧
    // 断言只用 done 之前到达的帧，不受此处影响。
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match tokio::time::timeout_at(drain_deadline, next_ws_frame(&mut ws)).await {
            Ok(ev) => {
                if ev["method"] == "turn.append" && ev["params"]["session_id"] == key.as_str() {
                    let texts = append_frame_texts(&ev);
                    if !texts.is_empty() {
                        frames.push(texts);
                    }
                }
            }
            Err(_) => break,
        }
    }

    // 核心断言：**多帧**、且每帧只是回合的一部分——不是一次整块。
    assert!(
        frames.len() >= 2,
        "the turn must stream as multiple turn.append frames, got {frames:?}"
    );
    let all: String = frames.concat().concat();
    assert_eq!(
        all, "drip0 drip1 drip2 ",
        "frames must carry the partial-stream chunks in order, each exactly once: {frames:?}"
    );

    // 快照面对账：transcript = prompt + 3 个内容条目（不多不少——完整
    // assistant 帧的文本块被 driver 跳过，无重复）。
    let detail: serde_json::Value = cli
        .get(&detail_url)
        .send()
        .await
        .expect("final detail")
        .json()
        .await
        .expect("final json");
    let entries = detail["entries"].as_array().expect("entries");
    let contents: Vec<&str> = entries
        .iter()
        .filter_map(|e| e["content"].as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["drip", "drip0 ", "drip1 ", "drip2 "],
        "transcript must reconcile with the streamed frames, no duplication: {contents:?}"
    );
}

/// fix-webui-streaming-liveness 6.2：native 会话在 webui 面逐 delta 到达。
/// SSE 假上游按 400ms 间隔吐 4 个 text_delta——native pump 逐 delta 落账并
/// 广播（不再攒到回合边界整块 flush），webui WS 在「🗒 turn summary」收尾帧
/// 之前收到**多帧**增量；最终 transcript 与帧序列对账一致、逐字各一次。
/// （IM 桥面的粒度一致性由 agent_backend 的对账单测 2.2 承载；本用例钉
/// detached 拓扑下 webui 面的实时性。）
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn native_turn_streams_deltas_to_the_webui() {
    let sb = Sandbox::new("testsuite_e2e", "native-drip");
    // 本地 SSE 上游：4 个 text_delta，间隔 400ms（每个落在独立合并窗）。
    let upstream = spawn_sse_stub_upstream(&["alpha ", "beta ", "gamma ", "end"], 400).await;
    let base = format!("http://127.0.0.1:{upstream}");
    let cli = http_client();
    let _core = sb.spawn_core_extra(&[
        ("SEBAS_AGENT_PROVIDER_BASE_URL", base.as_str()),
        ("SEBAS_AGENT_PROVIDER_API_KEY", "sk-sandbox-native"),
    ]);
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 先连 WS 并等核心通道订阅上线（同 claude drip 用例：resync 即信号，
    // 4s 未到则订阅已在前置检查期间上线）。
    let mut ws: WsStream = ws_connect(&sb.webui_url()).await;
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let ev = next_ws_frame(&mut ws).await;
            if ev["method"] == "session.resync" {
                break;
            }
        }
    })
    .await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hi", "agent": "native" }),
    )
    .await
    .expect("create native session");
    assert_eq!(status, 201, "native spawn must not be rejected: {body}");
    let key = body["key"].as_str().expect("key").to_string();

    // 读帧到收尾标记（🗒 turn summary 条目，native 回合结束才落账）。
    let mut frames: Vec<Vec<String>> = Vec::new();
    let mut mid_turn: Option<String> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        let ev = tokio::time::timeout_at(deadline, next_ws_frame(&mut ws))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for native turn frames"));
        if ev["method"] != "turn.append" || ev["params"]["session_id"] != key.as_str() {
            continue;
        }
        let texts = append_frame_texts(&ev);
        if texts.is_empty() {
            continue;
        }
        if texts.iter().any(|t| t.contains("turn summary")) {
            // 收尾帧可能同时捎带同一合并窗内的最后一个 delta——先入账再收。
            let tail: Vec<String> = texts
                .into_iter()
                .filter(|t| !t.contains("turn summary"))
                .collect();
            if !tail.is_empty() {
                frames.push(tail);
            }
            break;
        }
        if mid_turn.is_none() {
            // 首 delta 帧到达即抓 HTTP 快照：transcript 只有已到部分
            // ——正文在回合内可见，不是收尾一次性整块。
            let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
            let snap: serde_json::Value = cli
                .get(&detail_url)
                .send()
                .await
                .expect("mid-turn detail")
                .json()
                .await
                .expect("mid-turn json");
            let streamed: String = snap["entries"]
                .as_array()
                .map(|b| {
                    b.iter()
                        .filter(|e| e["kind"].as_str() == Some("content"))
                        .filter_map(|e| e["content"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            assert!(
                streamed.contains("alpha ") && !streamed.contains("end"),
                "mid-turn transcript must show the leading deltas only: {streamed:?}"
            );
            mid_turn = Some(streamed);
        }
        frames.push(texts);
    }

    // 多帧逐 delta；帧内容拼起来恰好是上游的 4 段（各一次、有序）。
    assert!(
        frames.len() >= 2,
        "native deltas must arrive as multiple turn.append frames, got {frames:?}"
    );
    let all: String = frames.concat().concat();
    assert_eq!(
        all, "alpha beta gamma end",
        "frames must carry every delta exactly once, in order: {frames:?}"
    );
    assert!(mid_turn.is_some(), "the mid-turn snapshot must have been taken");

    // 快照面对账：transcript 与帧序列完全一致（📖 过程痕迹之外，正文四段）。
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let detail: serde_json::Value = cli
        .get(&detail_url)
        .send()
        .await
        .expect("final detail")
        .json()
        .await
        .expect("final json");
    let transcript: String = detail["entries"]
        .as_array()
        .map(|b| {
            b.iter()
                .filter(|e| e["kind"].as_str() == Some("content"))
                .filter(|e| !e["content"].as_str().unwrap_or("").contains("turn summary"))
                .filter_map(|e| e["content"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    assert_eq!(
        transcript, "alpha beta gamma end",
        "native transcript must reconcile with the streamed deltas exactly once: {transcript:?}"
    );
    assert!(
        detail["entries"]
            .as_array()
            .is_some_and(|b| b.iter().any(|e| e["content"]
                .as_str()
                .is_some_and(|c| c.contains("turn summary")))),
        "the turn-end summary entry must close the turn: {detail}"
    );
}

/// fix-webui-streaming-liveness 6.3（HTTP 级钉住的两条）：大会话（1200 条
/// transcript）下 `GET /api/summary` 恒为 KB 级（<100KB）且不内嵌 entries，
/// 响应体积不随流式条目数线性增长（流式中途与收尾后两次采样同为 KB 级）；
/// 正文只走 detail 游标路径——`entries_after=<cursor>` 只回尾部条目。
/// （headless Chrome 的 long-task 采样不在 HTTP 断言能力内，另行执行。）
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn summary_stays_small_while_transcript_is_large() {
    let sb = Sandbox::new("testsuite_e2e", "summary-flood");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "flood", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let summary_url = format!("{}/api/summary", sb.webui_url());
    let hint = sb.path.clone();

    async fn summary_size(cli: &reqwest::Client, url: &str) -> (usize, serde_json::Value) {
        let resp = cli.get(url).send().await.expect("summary");
        let bytes = resp.bytes().await.expect("summary body");
        let size = bytes.len();
        let v = serde_json::from_slice(&bytes).expect("summary json");
        (size, v)
    }

    // 回合收尾：1200 条全部落账。1200 个 back-to-back 的 delta 在几秒内跑完
    // ——「working + 已有 content」的采样窗可能比 wait_for 的轮询间隔还短，
    // 所以这里的判据只看条目数（done 同样满足），不与状态位竞速。
    wait_for(
        "flood turn to land 1201 entries",
        Duration::from_secs(90),
        &hint,
        {
            let cli = cli.clone();
            let url = detail_url.clone();
            move || {
                let cli = cli.clone();
                let url = url.clone();
                Box::pin(async move {
                    let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                    let n = v["entries"]
                        .as_array()
                        .map(|b| b.len())
                        .unwrap_or(0);
                    (n >= 1201).then_some(v)
                })
            }
        },
    )
    .await;

    // 收尾后采样：summary 体积与条目数无关（1200 条仍 <100KB）。
    let (final_size, final_summary) = summary_size(&cli, &summary_url).await;
    assert!(
        final_size < 100 * 1024,
        "post-turn summary must stay <100KB with 1200 entries, got {final_size} bytes"
    );
    assert_eq!(
        final_summary["active_session_key"].as_str(),
        Some(key.as_str()),
        "summary must still name the focused session"
    );
    assert!(
        final_summary["active_session"].get("entries").is_none(),
        "summary must not embed the transcript (D3 split)"
    );

    // 正文只走 detail：全量 ≥1200 条；游标增量只回尾部（响应有界）。
    let full: serde_json::Value = cli
        .get(&detail_url)
        .send()
        .await
        .expect("full detail")
        .json()
        .await
        .expect("full json");
    let entries = full["entries"].as_array().expect("entries");
    assert!(
        entries.len() >= 1200,
        "the flood turn must land >=1200 entries, got {}",
        entries.len()
    );
    let cursor = entries.len() as u64 - 300;
    let tail: serde_json::Value = cli
        .get(format!("{detail_url}?entries_after={cursor}"))
        .send()
        .await
        .expect("cursor detail")
        .json()
        .await
        .expect("cursor json");
    let tail_entries = tail["entries"].as_array().expect("tail entries");
    assert_eq!(
        tail_entries.len() as u64,
        entries.len() as u64 - 1 - cursor,
        "the cursor fetch must return exactly the entries after the cursor"
    );
    assert_eq!(
        tail_entries[0]["position"].as_u64(),
        Some(cursor + 1),
        "cursor fetch starts strictly after the cursor"
    );
    assert_eq!(
        tail_entries.last().unwrap()["position"].as_u64(),
        Some(entries.len() as u64 - 1),
    );
}

// ---------------------------------------------------------------------------
// fix-webui-qa-defects 2.4 的进程级回归（本 change
// fix-webui-approval-restore-and-session-identity 7.2 遗留债）：归档 → 恢复 →
// rail 可见、转写完整、History 清空——纯 HTTP 面钉同一契约（浏览器级旅程由
// testsuite-webui 的 archive-restore.spec / archive-identity.spec 承担）。
// ---------------------------------------------------------------------------

/// 归档恢复全契约：完整回合 → 归档（活动列表退场、条目带全量转写快照 +
/// agent 身份）→ 恢复（行重建回原项目、转写完整、History 清空、可继续对话、
/// 身份原样带回）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn archive_restore_rebuilds_row_transcript_and_clears_history() {
    let sb = Sandbox::new("testsuite_e2e", "archive-restore");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 完整回合先落地：归档快照与恢复重建都必须带走它。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "project_id": project_id.clone(),
            "prompt": "archive-me",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_turn_done(&cli, &sb, &detail_url).await;
    let (_status, before) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail before archive");
    let entries_before = before["entries"].as_array().expect("entries").len();
    assert!(entries_before > 0, "the turn must have landed: {before}");

    // 归档：行退出活动列表；条目携带全量转写快照 + 归档时刻的身份
    // （本 change 3.1：agent_kind 随 SessionInfo 落档）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/archive", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("archive");
    assert_eq!(status, 200, "archive: {body}");
    assert_eq!(body["status"], "archived", "{body}");

    let (_, list) = get_json_status(&cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list after archive");
    assert!(
        list["recent_sessions"]
            .as_array()
            .map_or(true, |rows| rows
                .iter()
                .all(|r| r["encoded_key"].as_str() != Some(key.as_str()))),
        "the archived session must leave the active list: {list}"
    );

    let (_, archive) = get_json_status(&cli, &format!("{}/api/archive", sb.webui_url()))
        .await
        .expect("archive list after archive");
    let entries = archive["archived_sessions"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "History holds exactly the new entry: {archive}");
    let entry = &entries[0];
    // axum percent-decodes path params, so the entry stores the RAW key
    // (literal NUL) while the create response carries the encoded form.
    let raw_key = key.replace("%00", "\u{0}");
    assert_eq!(entry["session_key"].as_str(), Some(raw_key.as_str()), "{entry}");
    assert_eq!(
        entry["transcript"].as_array().map(Vec::len),
        Some(entries_before),
        "the snapshot must carry the full transcript: {entry}"
    );
    assert_eq!(
        entry["agent_kind"].as_str(),
        Some("claude"),
        "the entry must carry the agent identity: {entry}"
    );

    // 恢复：行重建回原项目（rail 可见）、转写完整、History 清空。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/restore", sb.webui_url()),
        serde_json::json!({}),
    )
    .await
    .expect("restore");
    assert_eq!(status, 200, "restore: {body}");
    assert_eq!(body["status"], "restored", "{body}");

    let (_, list) = get_json_status(&cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list after restore");
    let row = list["recent_sessions"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["encoded_key"].as_str() == Some(key.as_str()))
        .unwrap_or_else(|| panic!("the rebuilt session must be listed again: {list}"));
    assert_eq!(
        row["project_id"].as_str(),
        Some(project_id.as_str()),
        "the row must be back under its original project: {row}"
    );

    let (_status, after) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after restore");
    assert_eq!(
        after["entries"].as_array().map(Vec::len),
        Some(entries_before),
        "the rebuilt session must expose the SAME transcript: {after}"
    );
    // （3.2）身份随恢复链路原样带回。
    assert_eq!(
        after["agent_kind"].as_str(),
        Some("claude"),
        "the restored session must keep its agent identity: {after}"
    );

    let (_, archive) = get_json_status(&cli, &format!("{}/api/archive", sb.webui_url()))
        .await
        .expect("archive list after restore");
    assert!(
        archive["archived_sessions"]
            .as_array()
            .map_or(true, |rows| rows.is_empty()),
        "History must be cleared after a successful restore: {archive}"
    );

    // 可写：恢复后的会话照常对话（Resume 路径完成完整回合）。
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions/{key}/message", sb.webui_url()),
        serde_json::json!({ "message": "still here" }),
    )
    .await
    .expect("message after restore");
    assert_eq!(status, 200, "the restored session must be writable: {body}");
    wait_turn_done(&cli, &sb, &detail_url).await;
    let (_status, grown) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after follow-up");
    assert!(
        grown["entries"].as_array().expect("entries").len() > entries_before,
        "the follow-up must append on top of the restored transcript: {grown}"
    );
}

// ---------------------------------------------------------------------------
// fix-webui-qa-defects-round4 的进程级回归：升级击杀保留会话（1.2/1.3）、
// 接收回执阶段可取消（2.3/2.4）、归档恢复命名来源保留（3.1）。单元/路由层
// 契约已由 sebas-dispatch / sebas-webui 各自的测试钉住；这里补「真 core 子
// 进程 + 真 fake-claude 驱动」一层的 API 面证据。浏览器级 GUI 旅程由
// testsuite-webui 承担（review 阶段单独执行，不在本套件）。
// ---------------------------------------------------------------------------

/// （round4 1.2/1.3，session-lifecycle「Escalation kill keeps the session
/// browsable」+ acp-driver「Escalation finalizes the turn without erasing the
/// session」）P0 进程级复现：fake-claude 在 `hang` 后沉默（活着、探测照答），
/// `SEBAS_HANG_TIMEOUT_SECS=2` 让击杀阶梯秒级走完；`--ignore-interrupt` 让
/// fake 扛住 interrupt → 驱动必走完整阶梯（interrupt×3 → disconnect →
/// drop ≈SIGKILL）并以 terminal Error 收尾。修复前该会话从 /api/sessions
/// 整行消失、详情 404；修复后必须：列表含行（dormant）、详情 200 且末回合
/// 是携带升级原因的错误条目、击杀前产物保留可回看、排队提交经
/// `session.pending_dropped` 逐条如实上报后清空、全程无 `session.removed`。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn escalation_kill_keeps_the_session_on_the_api_surface() {
    let sb = Sandbox::new("testsuite_e2e", "escalation-kill");
    sb.append_acp_args(&["--ignore-interrupt"]);
    let cli = http_client();
    // 2s hang 阈值：阶梯 ≈ 2s 探测 + 3×2s interrupt 宽限 + 5s SIGKILL 宽限
    // ≈ 13s；给 90s 余量等退役（首回合的正常输出远快于阈值，不误杀）。
    let _core = sb.spawn_core_extra(&[("SEBAS_HANG_TIMEOUT_SECS", "2")]);
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    // 先落一个完整回合：击杀前的产出必须保留（spec「the operator can see
    // what happened and what was produced before the hang」）。
    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "project_id": project_id.clone(),
            "prompt": "hello",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_turn_done(&cli, &sb, &detail_url).await;
    let (_status, before) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail before the hang");
    let entries_before = before["entries"].as_array().expect("entries").len();
    assert!(entries_before > 0, "the first turn must have landed: {before}");

    // hang 提交：fake 永不再出帧 → 接收回执相位一直挂着，直到阶梯击杀。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "hang" }),
    )
    .await
    .expect("hang message");
    assert_eq!(status, 200, "hang message: {body}");

    // WS 先连上再排队提交：pending_dropped 帧 = not-executed 上报的进程级
    // 证据（上报发生在退役之前，同一连接 wire 保序，必在 dormant Updated
    // 之前到达）。阶梯静默窗 ≈ 11-13s，超过 next_ws_frame 的 15s 单帧超时
    // 余量太窄——这里用放宽版读帧（唯一 deadline 由本任务掌控）。
    let mut ws: WsStream = ws_connect(&sb.webui_url()).await;
    let ws_task = tokio::spawn(async move {
        use futures_util::StreamExt;
        // 放宽版读帧：唯一 deadline 由调用方给定（阶梯静默窗 ≈ 11-13s，
        // next_ws_frame 的 15s 单帧超时余量太窄）。
        async fn read_frame(
            ws: &mut WsStream,
            deadline: tokio::time::Instant,
        ) -> Option<serde_json::Value> {
            loop {
                let msg = tokio::time::timeout_at(deadline, ws.next())
                    .await
                    .ok()??
                    .ok()?;
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        return serde_json::from_str::<serde_json::Value>(&text).ok();
                    }
                    _ => continue,
                }
            }
        }
        let mut dropped_frame = None;
        let mut removed_frame = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            // pending_dropped 之后再排 5s：给（不该出现的）session.removed
            // 留出到达窗口，然后收尾。
            let step = if dropped_frame.is_some() {
                tokio::time::Instant::now() + Duration::from_secs(5)
            } else {
                deadline
            };
            match read_frame(&mut ws, step).await {
                Some(ev) => {
                    let method = ev["method"].as_str().unwrap_or_default().to_string();
                    if method == "session.removed" {
                        removed_frame = Some(ev);
                        break;
                    }
                    if method == "session.pending_dropped" {
                        dropped_frame = Some(ev);
                    }
                }
                None => break,
            }
        }
        (dropped_frame, removed_frame)
    });

    // 击杀前排队第二条提交：接收回执相位 = 在飞 → 必须入队（200），且在
    // 详情 pending 里可见（seed_card 在提交返回前同步落账，无竞态窗口）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "queued while hung" }),
    )
    .await
    .expect("queue during hang");
    assert_eq!(
        status, 200,
        "a submission during the hang must be accepted: {body}"
    );
    wait_for(
        "the queued submission to show up in the detail pending view",
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
                    (v["pending"].as_array()?.len() == 1).then_some(())
                })
            }
        },
    )
    .await;

    // 等阶梯走完、terminal Error 退役：行转 dormant（修复前：整行消失）。
    wait_for(
        "the escalation kill to retire the row to dormant",
        Duration::from_secs(90),
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
                    (v["status"].as_str() == Some("dormant")).then_some(())
                })
            }
        },
    )
    .await;

    // 列表仍含该行（dormant 态），行名保留首 prompt 预览——不退化为短 id。
    let (_status, list) = get_json_status(&cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list after the kill");
    let row = list["recent_sessions"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["encoded_key"].as_str() == Some(key.as_str()))
        .unwrap_or_else(|| panic!("the killed session must stay listed: {list}"));
    assert_eq!(row["status"], "dormant", "the retired row reports dormant: {row}");
    assert_eq!(
        row["prompt_preview"], "hang",
        "the row name must survive card-state teardown: {row}"
    );

    // 详情 200：击杀前产物保留 + hang 的 prompt 条目 + 携带升级原因的错误
    // 条目收尾；pending 已随退役清空。
    let (status, after) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the kill");
    assert_eq!(status, 200, "the retired session must stay retrievable");
    let entries = after["entries"].as_array().expect("entries");
    assert!(
        entries.len() >= entries_before + 2,
        "pre-kill transcript + the hang prompt + the error entry must all be there: {after}"
    );
    assert!(
        entries[..entries_before]
            .iter()
            .any(|e| e["kind"].as_str() == Some("prompt")),
        "the pre-kill turn must remain browsable: {after}"
    );
    let last = entries.last().expect("non-empty after the kill");
    assert_eq!(
        last["element_type"], "error",
        "the killed turn must be finalized with a visible error entry: {after}"
    );
    assert!(
        last["content"].as_str().map_or(false, |c| c.contains("agent hung")),
        "the escalation cause must ride the error entry: {last:?}"
    );
    assert!(
        after["pending"].as_array().map_or(false, Vec::is_empty),
        "the released queue must be empty on the retired row: {after}"
    );

    // WS 证据：pending_dropped 携带被释放提交原文；且无 session.removed
    // （记录移除不得被广播——Removed 只属于操作者显式 close）。
    let (dropped_frame, removed_frame) = ws_task.await.expect("ws task alive");
    let dropped_frame = dropped_frame
        .expect("pending_dropped must be announced: the drop is never silent");
    assert_eq!(
        dropped_frame["params"]["session_id"],
        key.as_str(),
        "{dropped_frame}"
    );
    assert_eq!(
        dropped_frame["params"]["dropped"][0]["text"],
        "queued while hung",
        "the queued submission must be reported as not executed: {dropped_frame}"
    );
    assert!(
        removed_frame.is_none(),
        "a session.removed frame must NOT be broadcast for a terminal teardown: {removed_frame:?}"
    );

    // 驱动日志：阶梯确实走过（不是别的原因杀的）。
    let core_log = std::fs::read_to_string(&sb.core_log).unwrap_or_default();
    assert!(
        core_log.contains("escalating (interrupt 1/3)"),
        "the kill ladder must have run: {}",
        sb.core_log.display()
    );

    // 「下一消息 fresh spawn」：对退役记录再发一条消息——既有 Dormant→resume
    // 路径（load 被拒时诚实回退）孵化全新会话并完成完整回合；保留记录 ≠
    // 留下一个不能再说话的坑。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "hello again" }),
    )
    .await
    .expect("follow-up after the kill");
    assert_eq!(
        status, 200,
        "the retired record must stay messageable (fresh spawn): {body}"
    );
    wait_turn_done(&cli, &sb, &detail_url).await;
    let (_status, grown) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the follow-up");
    assert!(
        grown["entries"].as_array().expect("entries").len() > entries.len(),
        "the fresh turn must append on top of the preserved transcript: {grown}"
    );
}

/// （round4 2.3/2.4，agent-workbench「accepted receipt without agent output
/// offers stop」的引擎半边进程级旅程）提交被接受、agent 首帧未落的接收回执
/// 相位里，取消必须可达成（修复前该相位 interrupt 被 409 Idle 顶回，操作员
/// 在 600s 看门狗兜底前无自助手段）：cancel 200 → 回合以「回合被停止」错误
/// 条目收尾（驱动按既有取消语义重生子进程，会话保持 active）→ 会话仍可
/// 继续对话。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn receipt_phase_cancel_stops_the_turn_and_the_session_stays_talkative() {
    let sb = Sandbox::new("testsuite_e2e", "receipt-cancel");
    let cli = http_client();
    // 默认 5 分钟 hang 阈值：取消必须远早于阶梯，可靠复现「回执窗口内的
    // 自助停止」。
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "project_id": project_id.clone(),
            "prompt": "hello",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_turn_done(&cli, &sb, &detail_url).await;

    // hang 提交：提交已接受、fake 永不出帧 → 接收回执相位在 API 面可见
    // （turn_engaged=true 且最新转录条目仍是操作员提交）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "hang" }),
    )
    .await
    .expect("hang message");
    assert_eq!(status, 200, "hang message: {body}");
    wait_for(
        "the receipt phase to show on the API face",
        Duration::from_secs(15),
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
                    let entries = v["entries"].as_array()?;
                    let last = entries.last()?;
                    let receipt = v.get("turn_engaged").and_then(serde_json::Value::as_bool)
                        == Some(true)
                        && last["kind"].as_str() == Some("prompt")
                        && last["content"].as_str() == Some("hang");
                    receipt.then_some(())
                })
            }
        },
    )
    .await;

    // 停止：该相位必须可取消（修复前 409 Idle）。
    let resp = cli
        .post(format!("{detail_url}/cancel"))
        .send()
        .await
        .expect("cancel during the receipt phase");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("cancel body");
    assert_eq!(
        status,
        200,
        "the accepted-receipt phase must be cancellable, not 409: {body}"
    );
    assert_eq!(body["status"], "cancelled", "{body}");

    // 回合收尾：fake 被 interrupt 以错误结果退场 → 驱动按既有取消语义重生
    // 子进程并发布 Finished——回合以「回合被停止」错误条目收尾，会话保持
    // active（respawn 的活子进程，不是终态拆除）。
    wait_for(
        "the cancelled turn to settle with its stop entry",
        Duration::from_secs(30),
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
                    let settled = v["status_slug"].as_str() == Some("done")
                        && v.get("turn_engaged").is_none();
                    settled.then_some(())
                })
            }
        },
    )
    .await;
    let (_status, settled) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after cancel");
    let settled_entries = settled["entries"].as_array().expect("entries").len();
    assert!(
        settled["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|e| e["element_type"].as_str() == Some("error")),
        "the stopped turn must leave a visible error entry: {settled}"
    );

    // 会话仍可对话：下一条消息孵化全新会话并完成完整回合（「下一消息
    // fresh spawn」的可观察语义不变）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "hello again" }),
    )
    .await
    .expect("follow-up after cancel");
    assert_eq!(status, 200, "the session must stay talkative: {body}");
    wait_turn_done(&cli, &sb, &detail_url).await;
    let (_status, grown) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after the follow-up");
    assert!(
        grown["entries"].as_array().expect("entries").len() > settled_entries,
        "the follow-up turn must append on top of the preserved transcript: {grown}"
    );
}

/// （round4 3.1，project-session-actions「restore preserves the row name」）
/// 归档→恢复 API 面：命名来源（操作者 label + 首 prompt 预览）随归档快照
/// 落档、随恢复迁回——恢复后的行两个来源与归档前逐一相同，不退化为短 id。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn archive_restore_preserves_the_row_naming_sources() {
    let sb = Sandbox::new("testsuite_e2e", "restore-naming");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "project_id": project_id.clone(),
            "prompt": "run the release checklist",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_turn_done(&cli, &sb, &detail_url).await;

    // 给会话设一个操作者 label：两个命名来源同时在场（label 第一顺位、
    // preview 第二顺位），恢复后都必须原样回来。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/label"),
        serde_json::json!({ "label": "发布清单会话" }),
    )
    .await
    .expect("set label");
    assert_eq!(status, 200, "set label: {body}");

    // 归档前行命名面：preview 来自首条消息、label 来自操作者。
    let row_field = |list: &serde_json::Value, field: &str| {
        list["recent_sessions"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|r| r["encoded_key"].as_str() == Some(key.as_str()))
            .unwrap_or_else(|| panic!("the session must be listed: {list}"))[field]
            .clone()
    };
    let (_status, list) = get_json_status(&cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list before archive");
    assert_eq!(
        row_field(&list, "prompt_preview"),
        "run the release checklist",
        "the first prompt must name the row before archival: {list}"
    );
    assert_eq!(
        row_field(&list, "label"),
        "发布清单会话",
        "the operator label must be on the row before archival: {list}"
    );

    // 归档：条目携带两个命名来源（迁移载体）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/archive"),
        serde_json::json!({}),
    )
    .await
    .expect("archive");
    assert_eq!(status, 200, "archive: {body}");
    let (_, archive) = get_json_status(&cli, &format!("{}/api/archive", sb.webui_url()))
        .await
        .expect("archive list");
    let entry = &archive["archived_sessions"].as_array().expect("entries")[0];
    assert_eq!(
        entry["prompt_preview"].as_str(),
        Some("run the release checklist"),
        "the snapshot must carry the first-prompt preview: {entry}"
    );
    assert_eq!(
        entry["operator_label"].as_str(),
        Some("发布清单会话"),
        "the snapshot must carry the operator label: {entry}"
    );

    // 恢复：两个命名来源迁回映射——行与归档前同名，绝不退化短 id。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/restore"),
        serde_json::json!({}),
    )
    .await
    .expect("restore");
    assert_eq!(status, 200, "restore: {body}");
    let (_status, list) = get_json_status(&cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list after restore");
    assert_eq!(
        row_field(&list, "label"),
        "发布清单会话",
        "the operator label must survive the restore: {list}"
    );
    assert_eq!(
        row_field(&list, "prompt_preview"),
        "run the release checklist",
        "the first-prompt preview must survive the restore: {list}"
    );
    assert_eq!(
        row_field(&list, "status"),
        "dormant",
        "the restored row is a dormant record: {list}"
    );

    // 恢复后的会话照常可写（顺带钉住恢复行的 detail 可达性）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/message"),
        serde_json::json!({ "message": "still named" }),
    )
    .await
    .expect("message after restore");
    assert_eq!(status, 200, "the restored session must be writable: {body}");
    wait_turn_done(&cli, &sb, &detail_url).await;
}

// ---------------------------------------------------------------------------
// fix-webui-qa-defects-round5 的进程级回归（round5 tasks 1.x / 3.x 的真进程
// 一层）：内嵌复合后端（`sebas core --webui`，DualSessionBackend = acp 桥 +
// native）与 detached 两进程形态下的 pending 管理面可达 + 类型化拒绝透传
// （1.1/1.2），以及 label 写入的实时广播与行投影（3.1）。路由层 fake 已钉
// 拒绝映射（sebas-webui api.rs）、引擎层已钉 label 发布
// （sebas-dispatch approval_restore_identity_test）、复合转发已钉
// （src/agent_backend.rs dual_pending_management_routes_by_key）；这里补
// 「真 core 进程 + 真 HTTP/WS 面」的证据。浏览器级 GUI 旅程（重命名对话框、
// rail 行名免刷新）由 testsuite-webui / 主 agent GUI 回归承担。
// ---------------------------------------------------------------------------

/// 在 `/ws` 帧流里等目标会话携带目标 `label` 的下一条 `session.updated`
/// （跳过无关帧与 label 未变化的**回水帧**：resync 护栏放行后，建会话/回合
/// 收敛期间的相位帧仍会在读帧前排队，它们 label 恒为写入前的旧值，必须跳过
/// 直到目标写入的帧到达；单帧 15s 超时由 [`next_ws_frame`] 兜底，循环上限防
/// 无限流）。
async fn wait_session_updated_with_label(
    ws: &mut WsStream,
    key: &str,
    label: Option<&str>,
) -> serde_json::Value {
    for _ in 0..100 {
        let frame = next_ws_frame(ws).await;
        let method = frame["method"].as_str().unwrap_or_default();
        let sid = frame["params"]["session_id"].as_str().unwrap_or_default();
        let label_matches = match label {
            Some(expected) => frame["params"]["label"].as_str() == Some(expected),
            None => frame["params"]["label"].is_null(),
        };
        if method == "session.updated" && sid == key && label_matches {
            return frame;
        }
    }
    panic!("no session.updated frame for {key} with label {label:?} arrived");
}

/// `GET /api/sessions` 里目标会话的行投影（rail 行名重取的数据源）。
async fn listed_row(cli: &reqwest::Client, sb: &Sandbox, key: &str) -> serde_json::Value {
    let (_status, list) = get_json_status(cli, &format!("{}/api/sessions", sb.webui_url()))
        .await
        .expect("session list");
    list["recent_sessions"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["encoded_key"].as_str() == Some(key))
        .cloned()
        .unwrap_or_else(|| panic!("the session must be listed: {list}"))
}

/// 忙中把两条提交排进待执行队列（首回合 streaming 期间），返回
/// (detail_url, 先提交的 id, 后提交的 id)。首回合用「stream」场景
/// （5 帧 × 250ms，与 turn_queue_timing 同款节奏且可应答 watchdog 探测），
/// 内容帧落地后忙中提交确定性入队。
async fn queue_two_submissions(
    cli: &reqwest::Client,
    base: &str,
    project_id: &str,
    hint: &std::path::Path,
) -> (String, u64, u64) {
    let (status, body) = post_json(
        cli,
        &format!("{base}/api/sessions"),
        serde_json::json!({
            "project_id": project_id,
            "prompt": "stream",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{base}/api/sessions/{key}");

    // 等首个 agent 内容帧落地（WORKING 真在跑）再提交——只认 kind=content
    // （seed 的 prompt 条目在 SEED 阶段就进 transcript，等非空会提前放行）。
    let content_cli = cli.clone();
    let content_url = detail_url.clone();
    let content_hint = hint.to_path_buf();
    wait_for("first turn content to stream", Duration::from_secs(40), hint, move || {
        let cli = content_cli.clone();
        let url = content_url.clone();
        Box::pin(async move {
            let v = cli
                .get(&url)
                .send()
                .await
                .ok()?
                .json::<serde_json::Value>()
                .await
                .ok()?;
            let streamed = v["entries"]
                .as_array()
                .is_some_and(|b| b.iter().any(|e| e["kind"].as_str() == Some("content")));
            streamed.then_some(())
        })
    })
    .await;

    for (tag, text) in [("b", "queued-round5-b"), ("c", "queued-round5-c")] {
        let (status, body) = post_json(
            cli,
            &format!("{detail_url}/message"),
            serde_json::json!({ "message": text }),
        )
        .await
        .unwrap_or_else(|e| panic!("submit {tag}: {e}"));
        assert_eq!(status, 200, "busy-time submission {tag} must be accepted: {body}");
    }

    // 两条都已在 pending 里（seed 同步落账；条目开始消费前有 ≈1s 窗口）。
    let poll_cli = cli.clone();
    let poll_url = detail_url.clone();
    let detail = wait_for("both submissions to ride in pending", Duration::from_secs(8), hint, move || {
        let cli = poll_cli.clone();
        let url = poll_url.clone();
        Box::pin(async move {
            let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
            let n = v["pending"].as_array().map(Vec::len)?;
            (n == 2).then_some(v)
        })
    })
    .await;
    let pending = detail["pending"].as_array().expect("pending list");
    let id_of = |text: &str| {
        pending
            .iter()
            .find(|p| p["text"].as_str() == Some(text))
            .unwrap_or_else(|| panic!("{text} must be in pending: {pending:?}"))["id"]
            .as_u64()
            .expect("pending id")
    };
    (detail_url, id_of("queued-round5-b"), id_of("queued-round5-c"))
}

/// （round5 1.1/1.2，core-session-channel「Session drive methods」内嵌场景）
/// 裸 core 内嵌 WebUI（`sebas core --webui`，复合后端 = acp 桥承载会话）下，
/// 排队提交的重排/移除必须到达承载子后端并成功，类型化拒绝原样透传
/// （unknown 404 / out_of_range 400），错误文案绝不自称「核心不可达」。
/// 修复前三个管理调用全部落复合层 trait 默认实现恒 503、文案谎报可达性。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn pending_management_reaches_the_host_backend_in_embedded_shape() {
    let sb = Sandbox::new("testsuite_e2e", "embedded-pending");
    sb.slow_fake_agent(400);
    let cli = http_client();
    let (_core, dashboard) = sb.spawn_core_inprocess_webui(&[]);
    let base = format!("http://127.0.0.1:{dashboard}");
    let hint = sb.path.clone();

    // 内嵌形态的 dashboard 端口是旗标指定的 probed 端口（≠ sb.webui_port）。
    let health_cli = cli.clone();
    let health_url = base.clone();
    let health_hint = hint.clone();
    wait_for("in-process webui health", Duration::from_secs(30), &health_hint, move || {
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

    // 项目注册走 dashboard 面（workspace root 已钉在沙箱目录，注册必定界内）。
    let (pstatus, pbody) = post_json(
        &cli,
        &format!("{base}/api/projects"),
        serde_json::json!({ "path": support::forward_slash(&sb.path) }),
    )
    .await
    .expect("register scene project");
    assert_eq!(pstatus, 201, "register scene project: {pbody}");
    let project_id = pbody["id"].as_str().expect("project id").to_string();

    let (detail_url, id_b, id_c) = queue_two_submissions(&cli, &base, &project_id, &hint).await;

    // 重排：queued-c 移到队首 → 200 且响应携带重排后的全量队列
    // （修复前：复合层缺转发恒 503「操作不可用: 此后端不承载待执行队列」，
    // 且旧文案自称「核心不可达」）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_c}/move"),
        serde_json::json!({ "to_index": 0 }),
    )
    .await
    .expect("move queued-c");
    assert_eq!(status, 200, "move must reach the acp bridge: {body}");
    assert_eq!(body["status"], "moved", "{body}");
    let moved = body["pending"].as_array().expect("pending after move");
    assert_eq!(moved[0]["text"], "queued-round5-c", "moved entry must lead: {body}");
    assert_eq!(moved[1]["text"], "queued-round5-b", "{body}");

    // 移除：queued-b 出队 → 200，剩余队列只含 queued-c。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_b}/remove"),
        serde_json::json!({}),
    )
    .await
    .expect("remove queued-b");
    assert_eq!(status, 200, "remove must reach the acp bridge: {body}");
    assert_eq!(body["status"], "removed", "{body}");
    let left = body["pending"].as_array().expect("pending after remove");
    assert_eq!(left.len(), 1, "{body}");
    assert_eq!(left[0]["text"], "queued-round5-c", "{body}");

    // 类型化拒绝透传：unknown id → 404（文案点名「不存在」，绝不 5xx、
    // 绝不自称不可达）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/9999/remove"),
        serde_json::json!({}),
    )
    .await
    .expect("remove unknown id");
    assert_eq!(status, 404, "unknown id must pass through typed: {body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("待执行提交不存在"), "{body}");
    assert!(!message.contains("核心不可达"), "{body}");

    // 越界重排 → 400（目标位置越界），同样诚实。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_c}/move"),
        serde_json::json!({ "to_index": 9 }),
    )
    .await
    .expect("move out of range");
    assert_eq!(status, 400, "out-of-range must pass through typed: {body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("目标位置越界"), "{body}");
    assert!(!message.contains("核心不可达"), "{body}");

    // 全程 core 可达——以上失败没有一条可以伪装成「不可达」。
    let (_status, summary) = get_json_status(&cli, &format!("{base}/api/summary"))
        .await
        .expect("summary");
    assert_eq!(
        summary["reachability"]["ok"], true,
        "the core was reachable the whole time: {summary}"
    );
}

/// （round5 1.1「any deployment shape」的 detached 半边）独立 webui + core
/// 两进程形态下同一管理面契约：move/remove 经核心通道到达 core 侧引擎队列，
/// 成功与类型化拒绝同样保真。内嵌形态（上一条）钉复合转发；这里钉「任何
/// 部署形态」的另一半。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn pending_management_reaches_the_core_queue_in_detached_shape() {
    let sb = Sandbox::new("testsuite_e2e", "detached-pending");
    sb.slow_fake_agent(400);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (detail_url, id_b, id_c) =
        queue_two_submissions(&cli, &sb.webui_url(), &project_id, &sb.path).await;

    // 重排 → 200（队列经核心通道在 core 侧翻转）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_c}/move"),
        serde_json::json!({ "to_index": 0 }),
    )
    .await
    .expect("move queued-c");
    assert_eq!(status, 200, "move must reach the core queue: {body}");
    assert_eq!(body["pending"][0]["text"], "queued-round5-c", "{body}");

    // 移除 → 200。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_b}/remove"),
        serde_json::json!({}),
    )
    .await
    .expect("remove queued-b");
    assert_eq!(status, 200, "remove must reach the core queue: {body}");
    let left = body["pending"].as_array().expect("pending after remove");
    assert_eq!(left.len(), 1, "{body}");
    assert_eq!(left[0]["text"], "queued-round5-c", "{body}");

    // 类型化拒绝透传：unknown 404 / out_of_range 400，文案诚实。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/9999/remove"),
        serde_json::json!({}),
    )
    .await
    .expect("remove unknown id");
    assert_eq!(status, 404, "{body}");
    assert!(body["error"].as_str().unwrap_or_default().contains("待执行提交不存在"), "{body}");
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_c}/move"),
        serde_json::json!({ "to_index": 9 }),
    )
    .await
    .expect("move out of range");
    assert_eq!(status, 400, "{body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(message.contains("目标位置越界"), "{body}");
    assert!(!message.contains("核心不可达"), "{body}");
}

/// （round5 3.1，project-session-actions「label writes through any path
/// update the row live」）被接受的 label 写入必须作为会话更新事件到达已
/// 连接的客户端（rail 行名免刷新重取的触发器）——detached 形态下写入走
/// 核心通道、事件经同一通道折返 /ws。列表行投影的 label 字段（重取的读源）
/// 同步断言：设置与清空各走一轮。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn label_write_reaches_clients_as_session_update_without_reload() {
    let sb = Sandbox::new("testsuite_e2e", "label-liveness");
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    // 先连 /ws 再写 label：帧必须在既有连接上到达（免刷新的证据本体）。
    let mut ws = ws_connect(&sb.webui_url()).await;
    // webui 启动会与 core 的 socket bind 竞速——首个流式订阅尝试可能失败、
    // 经 1s 退避后才连上（claude_turn_streams 同款护栏）：`session.resync`
    // 随每次订阅快照到达，等到它才说明 forwarder 已订阅、后续变更必达；
    // 若 4s 内没等到（forwarder 在本连接订阅前就已连上，resync 已被广播
    // 消费），订阅同样已就绪，继续即可。
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let ev = next_ws_frame(&mut ws).await;
            if ev["method"] == "session.resync" {
                break;
            }
        }
    })
    .await;

    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({
            "project_id": project_id,
            "prompt": "hello",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    wait_turn_done(&cli, &sb, &detail_url).await;

    // API 路径写入（rail 对话框与它共用同一 label API）。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/label"),
        serde_json::json!({ "label": "发布清单会话" }),
    )
    .await
    .expect("set label");
    assert_eq!(status, 200, "set label: {body}");

    // 更新帧到达：相位帧（session_id + 四相位键）。round5 6.3 起载荷扩展
    // `label`（操作者命名随帧如实下发）——rail 的行名重取收窄直接消费该键
    // （帧 label ≠ 行已知 label 才调度），断键即静默失活，必须在 wire 上钉住。
    let frame = wait_session_updated_with_label(&mut ws, &key, Some("发布清单会话")).await;
    let params = &frame["params"];
    assert!(params["status_slug"].is_string(), "phase frame: {frame}");
    assert!(params["turn_engaged"].is_boolean(), "phase frame: {frame}");
    assert!(params["msg_count"].is_number(), "phase frame: {frame}");
    assert!(params["pending"].is_array(), "phase frame: {frame}");

    // 行投影（rail 重取的读源）已携带新 label。
    let row = listed_row(&cli, &sb, &key).await;
    assert_eq!(
        row["label"], "发布清单会话",
        "the accepted label must be on the row: {row}"
    );

    // 清空（API 置 null）同样广播 + 行投影回退。
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/label"),
        serde_json::json!({ "label": null }),
    )
    .await
    .expect("clear label");
    assert_eq!(status, 200, "clear label: {body}");
    let frame = wait_session_updated_with_label(&mut ws, &key, None).await;
    assert!(
        frame["params"]["label"].is_null(),
        "the cleared label must ride the frame as null (never a stale value): {frame}"
    );
    let row = listed_row(&cli, &sb, &key).await;
    assert!(
        row["label"].is_null(),
        "the cleared label must leave the row: {row}"
    );
}

// ---------------------------------------------------------------------------
// （fix-webui-qa-defects-round5 1.2 收口；断言按 6.1/3c review 的 wire 现状
// 校准）busy 会话的待执行管理面经裸 core 复合后端 + 真 HTTP 路径验证：
// 重排合法生效（200）、已投递 id 的 remove 得 AlreadyStarted 409；上层
// 「unknown 404 / out_of_range 400」已随 embedded-shape 测试覆盖；
// PriorityConflict 的 409 映射在 wire 优先入口缺位下由 api.rs 的
// FakeBackend 缝隙单测钉住。错误文案绝不在某层被「不可用」笼统顶替。
//
// 真实复现路径：核心通道端状态层 PendingOpError → InProcessBackend 透传
// → DualSessionBackend.route() → api.rs PendingReason → StatusCode。
// ---------------------------------------------------------------------------


/// 在 stream 场景下把三条**普通**提交排进 busy 会话的待执行栈，返回
/// (detail_url, 三条 id)。「busy 会话带待执行栈」且不需要优先项的场景
/// （如复合后端按 key 路由的对照面）从这里拿——全部普通项，id 精确匹配
/// 文本，没有 /btw 前缀干扰。
async fn queue_three_pending(
    cli: &reqwest::Client,
    base: &str,
    project_id: &str,
    hint: &std::path::Path,
) -> (String, u64, u64, u64) {
    let (status, body) = post_json(
        cli,
        &format!("{base}/api/sessions"),
        serde_json::json!({
            "project_id": project_id,
            "prompt": "stream",
            "agent": "claude"
        }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{base}/api/sessions/{key}");

    // 等首个 agent 内容帧落地（WORKING 真在跑）。
    let content_cli = cli.clone();
    let content_url = detail_url.clone();
    let content_hint = hint.to_path_buf();
    wait_for("first turn content to stream", Duration::from_secs(40), &content_hint, move || {
        let cli = content_cli.clone();
        let url = content_url.clone();
        Box::pin(async move {
            let v = cli
                .get(&url)
                .send()
                .await
                .ok()?
                .json::<serde_json::Value>()
                .await
                .ok()?;
            let streamed = v["entries"]
                .as_array()
                .is_some_and(|b| b.iter().any(|e| e["kind"].as_str() == Some("content")));
            streamed.then_some(())
        })
    })
    .await;

    // 三条普通提交入栈（会话在飞 → 全部 pending）。
    for (tag, text) in [
        ("b", "queued-round5-b"),
        ("c", "queued-round5-c"),
        ("p", "queued-round5-p"),
    ] {
        let (status, body) = post_json(
            cli,
            &format!("{detail_url}/message"),
            serde_json::json!({ "message": text }),
        )
        .await
        .unwrap_or_else(|e| panic!("submit {tag}: {e}"));
        assert_eq!(status, 200, "busy-time submission {tag} must be accepted: {body}");
    }

    // 三条都在 pending。
    let poll_cli = cli.clone();
    let poll_url = detail_url.clone();
    let detail = wait_for(
        "three submissions to ride in pending",
        Duration::from_secs(8),
        hint,
        move || {
            let cli = poll_cli.clone();
            let url = poll_url.clone();
            Box::pin(async move {
                let v = cli.get(&url).send().await.ok()?.json::<serde_json::Value>().await.ok()?;
                let n = v["pending"].as_array().map(Vec::len)?;
                (n == 3).then_some(v)
            })
        },
    )
    .await;
    let pending = detail["pending"].as_array().expect("pending list");
    let id_of = |text: &str| {
        pending
            .iter()
            .find(|p| p["text"].as_str() == Some(text))
            .unwrap_or_else(|| panic!("{text} must be in pending: {pending:?}"))["id"]
            .as_u64()
            .expect("pending id")
    };
    (detail_url, id_of("queued-round5-b"), id_of("queued-round5-c"), id_of("queued-round5-p"))
}

/// （fix-webui-qa-defects-round5 1.2；断言按 6.1/3c review 的 wire 现状校准）
/// 裸 core 复合后端下的待执行管理面：
///
/// - **重排诚实行为 (200)**：wire 上尚无优先提交入口（web_send_message →
///   submit_turn 恒 priority=false，/btw 的优先位只在 feishu 入站生效），
///   队列全普通项——move c → 0 是合法重排，得 200 且顺序生效。
///   PriorityConflict 的 409 映射由 api.rs 的 FakeBackend 缝隙单测钉住；
///   优先项 wire 入口落地后，本腿改回「越过优先项 must 409」。
/// - **AlreadyStarted (409)**：等 c/b/p 依序投递完（id_p 离开 pending）→
///   用陈旧的 p id 重试 remove → 状态层 delivered 集合里命中 →
///   AlreadyStarted → 409。
///
/// 错误文案点名原因（该提交已开始执行）且绝不出现「核心不可达」
/// 「操作不可用」等其它 5xx/4xx 顶替。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn pending_management_passes_typed_rejections_through_composite_409() {
    let sb = Sandbox::new("testsuite_e2e", "embedded-pending-409");
    sb.slow_fake_agent(400);
    let cli = http_client();
    let (_core, dashboard) = sb.spawn_core_inprocess_webui(&[]);
    let base = format!("http://127.0.0.1:{dashboard}");
    let hint = sb.path.clone();

    // 内嵌形态 dashboard 等就绪。
    let health_cli = cli.clone();
    let health_url = base.clone();
    let health_hint = hint.clone();
    wait_for("in-process webui health", Duration::from_secs(30), &health_hint, move || {
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
                .unwrap_or(false)
                .then_some(())
        })
    })
    .await;

    // 注册项目（workspace root = 沙箱目录，注册必定界内）。
    let (pstatus, pbody) = post_json(
        &cli,
        &format!("{base}/api/projects"),
        serde_json::json!({ "path": support::forward_slash(&sb.path) }),
    )
    .await
    .expect("register scene project");
    assert_eq!(pstatus, 201, "register scene project: {pbody}");
    let project_id = pbody["id"].as_str().expect("project id").to_string();

    // 三条普通入栈（wire 上无优先提交入口，见段落注释）。
    let (detail_url, _id_b, id_c, id_p) =
        queue_three_pending(&cli, &base, &project_id, &hint).await;

    // ── 重排诚实行为 → 200：队列全普通项，move c → 0 合法且顺序生效。──
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_c}/move"),
        serde_json::json!({ "to_index": 0 }),
    )
    .await
    .expect("move c to front");
    assert_eq!(
        status, 200,
        "reordering plain submissions must succeed on the wire, got {body}"
    );
    let (_, detail) = get_json_status(&cli, &detail_url)
        .await
        .expect("detail after move");
    let order: Vec<String> = detail["pending"]
        .as_array()
        .expect("pending list")
        .iter()
        .map(|p| p["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        order,
        ["queued-round5-c", "queued-round5-b", "queued-round5-p"],
        "move to front must take effect: {order:?}"
    );

    // ── AlreadyStarted → 409：p 以普通项身份依序投递。轮询直到 id_p 离开
    // pending（= 已 pop 投递），保留陈旧 id 重试 remove：delivered 集合命中
    // → AlreadyStarted → 409。文案诚实性断言落在本错误腿上。
    {
        let poll_cli = cli.clone();
        let poll_url = detail_url.clone();
        wait_for("p to be delivered (leaves pending)", Duration::from_secs(60), &hint, move || {
            let cli = poll_cli.clone();
            let url = poll_url.clone();
            Box::pin(async move {
                let v = cli
                    .get(&url)
                    .send()
                    .await
                    .ok()?
                    .json::<serde_json::Value>()
                    .await
                    .ok()?;
                let delivered = v["pending"]
                    .as_array()?
                    .iter()
                    .all(|p| p["id"].as_u64() != Some(id_p));
                delivered.then_some(())
            })
        })
        .await;
    }
    let (status, body) = post_json(
        &cli,
        &format!("{detail_url}/pending/{id_p}/remove"),
        serde_json::json!({}),
    )
    .await
    .expect("remove already-started id");
    assert_eq!(
        status, 409,
        "removing a delivered id must 409 (AlreadyStarted), got {body}"
    );
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("该提交已开始执行"),
        "AlreadyStarted message must name the cause: {message}"
    );
    assert!(!message.contains("核心不可达"), "{message}");
    assert!(!message.contains("操作不可用"), "{message}");

    // 全程 core 可达——以上两类失败没有一条可以伪装成「不可达」。
    let (_status, summary) = get_json_status(&cli, &format!("{base}/api/summary"))
        .await
        .expect("summary");
    assert_eq!(
        summary["reachability"]["ok"], true,
        "the core was reachable the whole time: {summary}"
    );
}

/// （fix-webui-qa-defects-round5 1.3，core-session-channel 末段：错误文案
/// 不再自称「核心不可达」）复合后端路由缺位的诚实失败 = HTTP 503 + 文案
/// 「操作不可用: 此后端不承载待执行队列」——直接打不归 webui 路由的会话：
/// 该键对应的 channel 后端无队列 / 该 channel 走 native 侧（不路由到 acp
/// 桥）。这里通过在 feishu channel 上不存在会话的 known-format key 命中
/// native 侧不可用分支（route() 路由到 NativeAgentBackend → trait 默认
/// Unavailable），文字契约由 api.rs 透传。
///
/// 不创建 feishu 会话（feishu 通道默认 disabled，避免与既有沙箱冲突）；
/// 只断言调用契约与文案形态——这与 1.2 表格里 FakeBackend 注入 Unavailable
/// 的接口层单测是同形但反向验证：真实 InProcessBackend/NativeAgentBackend
/// 路径里走出来的错误文本也得是「操作不可用」而非「核心不可达」。
/// 键必须是 `agent-` 形引用（is_native 谓词），否则落 ACP 桥得 404 Unknown。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn pending_management_unavailable_uses_honest_cause_text() {
    let sb = Sandbox::new("testsuite_e2e", "embedded-pending-unavailable");
    sb.slow_fake_agent(400);
    let cli = http_client();
    let (_core, dashboard) = sb.spawn_core_inprocess_webui(&[]);
    let base = format!("http://127.0.0.1:{dashboard}");
    let hint = sb.path.clone();

    let health_cli = cli.clone();
    let health_url = base.clone();
    let health_hint = hint.clone();
    wait_for("in-process webui health", Duration::from_secs(30), &health_hint, move || {
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
                .unwrap_or(false)
                .then_some(())
        })
    })
    .await;

    // 注册项目（让 create 不撞 scope 边界）。
    let (pstatus, pbody) = post_json(
        &cli,
        &format!("{base}/api/projects"),
        serde_json::json!({ "path": support::forward_slash(&sb.path) }),
    )
    .await
    .expect("register scene project");
    assert_eq!(pstatus, 201, "register scene project: {pbody}");
    let project_id = pbody["id"].as_str().expect("project id").to_string();

    // web channel 创建会话（走 acp 桥）→ 三条普通提交入栈，作为「acp 侧
    // 队列照常可用」的对照面（本用例不需要优先项）。
    let (detail_url, _id_b, _id_c, _id_p) =
        queue_three_pending(&cli, &base, &project_id, &hint).await;

    // 用合法编码 key 但绕开 webui 路由：直接对 feishu channel 编码键（沙箱
    // feishu disabled，route() 落到 NativeAgentBackend 的 trait 默认实现 →
    // Unavailable cause = "此后端不承载待执行队列"）。WebUI 路由 decode 该
    // key 仍合法（webui 不按 channel 过滤），仅后端不可用。
    //
    // 编码约定：「channel%00reference」；is_native（src/agent_backend.rs）
    // 只认 feishu + `agent-` 前缀才路由 native 侧——引用缺前缀会落 ACP 桥
    // 得 PendingRejected::Unknown（404），断言的 503/诚实文案就永不匹配
    // （round5 6.1 修的键形状），故取一个不存在的 `agent-` 形 feishu 引用。
    let feishu_encoded = "feishu%00agent-round5-native-pending";
    for (op, suffix) in [("remove", "/remove"), ("move", "/move")] {
        let url = format!("{base}/api/sessions/{feishu_encoded}/pending/1{suffix}");
        let body = if op == "move" {
            serde_json::json!({ "to_index": 0 })
        } else {
            serde_json::json!({})
        };
        let (status, body) = post_json(&cli, &url, body)
            .await
            .unwrap_or_else(|e| panic!("{op} feishu channel: {e}"));
        assert_eq!(
            status, 503,
            "{op} on a non-queue-holding channel must 503 (Unavailable), got {body}"
        );
        let message = body["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("操作不可用"),
            "{op} message must use the round5 honest cause prefix: {message}"
        );
        assert!(
            message.contains("此后端不承载待执行队列"),
            "{op} message must name the routing gap: {message}"
        );
        assert!(
            !message.contains("核心不可达"),
            "{op} message must NOT pretend the core is unreachable: {message}"
        );
    }

    // 同一会话 web channel 路径上仍然 200（控制：复合后端按 key 路由成功）。
    let (_status, summary) = get_json_status(&cli, &format!("{base}/api/summary"))
        .await
        .expect("summary");
    assert_eq!(
        summary["reachability"]["ok"], true,
        "the core was reachable the whole time: {summary}"
    );
    let (_status, list) =
        get_json_status(&cli, &format!("{base}/api/sessions"))
            .await
            .expect("session list");
    let in_list = list["recent_sessions"]
        .as_array()
        .expect("rows")
        .iter()
        .any(|r| {
            r["encoded_key"].as_str()
                == Some(detail_url.rsplit('/').next().unwrap_or(""))
        });
    assert!(in_list, "the queued session must still be reachable by webui list");
}

/// 轮询直到某进程日志同时出现全部给定片段（posture 告警在启动早期落盘，
/// reachable 之后理应已在——wait_for 只是给慢机器留余量）。
async fn wait_log_contains(what: &str, log: &Path, needles: &[&str]) {
    let log = log.to_path_buf();
    let log_hint = log.clone();
    let needles: Vec<String> = needles.iter().map(|s| (*s).to_string()).collect();
    wait_for(what, Duration::from_secs(20), &log_hint, move || {
        let log = log.clone();
        let needles = needles.clone();
        Box::pin(async move {
            let text = std::fs::read_to_string(&log).ok()?;
            needles
                .iter()
                .all(|n| text.contains(n.as_str()))
                .then_some(())
        })
    })
    .await;
}

/// close-acceptance-blind-spots 盲区 2 进程级 e2e（spec 场景「未覆盖的继承
/// env 触发告警」）：shell 导出 `ANTHROPIC_BASE_URL` / `ANTHROPIC_MODEL` 后，
/// core 与 standalone webui 的启动日志各出现一条 env posture WARN，逐变量
/// 点名「继承自 shell、将在 Claude 子进程生效」。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn env_posture_warn_names_inherited_anthropic_vars() {
    let sb = Sandbox::new("testsuite_e2e", "env-posture-warn");
    let cli = http_client();
    let inherited: &[(&str, &str)] = &[
        ("ANTHROPIC_BASE_URL", "https://corp-gw.internal/anthropic"),
        ("ANTHROPIC_MODEL", "corp-unrouted-model"),
    ];
    let _core = sb.spawn_core_extra(inherited);
    let _webui = sb.spawn_webui_extra(&sb.core_secret, inherited);
    wait_reachable(&cli, &sb).await;

    for (who, log) in [("core", sb.core_log.clone()), ("webui", sb.webui_log.clone())] {
        wait_log_contains(
            &format!("{who} 启动日志出现 env posture 告警并点名两个继承变量"),
            &log,
            &[
                "env posture",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_MODEL",
                "继承自 shell",
                "Claude 子进程生效",
            ],
        )
        .await;
    }
}

/// spec 场景「全覆盖时保持安静」的进程级反例：不继承任何被检变量时，core
/// 与 webui 的启动日志都不得出现 env posture 告警。七个被检变量显式置空
/// （空值 = 未继承），避免跑套件的 shell 恰好导出真实 `ANTHROPIC_*` 污染断言。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn env_posture_stays_silent_without_inherited_vars() {
    let sb = Sandbox::new("testsuite_e2e", "env-posture-silent");
    let cli = http_client();
    let blanks: &[(&str, &str)] = &[
        ("ANTHROPIC_BASE_URL", ""),
        ("ANTHROPIC_AUTH_TOKEN", ""),
        ("ANTHROPIC_MODEL", ""),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", ""),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", ""),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", ""),
        ("ANTHROPIC_SMALL_FAST_MODEL", ""),
    ];
    let _core = sb.spawn_core_extra(blanks);
    let _webui = sb.spawn_webui_extra(&sb.core_secret, blanks);
    wait_reachable(&cli, &sb).await;

    // posture 检测在启动早期执行；reachable 蕴含两边启动日志早已落盘。
    for (who, log) in [("core", &sb.core_log), ("webui", &sb.webui_log)] {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        assert!(
            !text.contains("env posture"),
            "{who} 启动日志不得出现 env posture 告警（无继承必须静默）:\n{text}"
        );
    }
}

// ── close-acceptance-blind-spots 1.3：fake-claude 全行为 mock 的进程级 e2e ──
//
// 场景用例即桩的验收（design Risks）：桩的行为由真实 core+webui 子进程反向
// 断言，桩坏了套件红、不静默假绿。全部走一次性沙箱，零真模型调用。

/// spec 场景「工具环场景」（close-acceptance-blind-spots 1.3）：tool-loop
/// 桩发出 tool_use → 泊车审批由**测试侧**经 webui HTTP 应答 allow_once →
/// tool_result → 环后正文 → Done。投影按 position 单调读出
/// 「tool_use（📖 Bash）→ 工具执行（✓ Bash）→ 后续正文」三段序列；
/// journal 逐行点名所用场景（design D4）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn tool_loop_scenario_projects_tool_use_execution_and_post_tool_text() {
    let sb = Sandbox::new("testsuite_e2e", "fake-tool-loop");
    let journal = sb.path.join("fake-claude-journal.jsonl");
    sb.append_acp_args(&[
        "--scenario",
        "tool-loop",
        "--journal",
        &support::forward_slash(&journal),
    ]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "run the loop", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    // 工具环的「测试侧」半边：等泊车审批出现在读模型，应答 allow_once。
    let hint = sb.path.clone();
    let request_id = wait_for(
        "parked tool-loop approval",
        Duration::from_secs(30),
        &hint,
        {
            let cli = cli.clone();
            let url = format!("{}/api/sessions/{key}/approvals", sb.webui_url());
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
                    let approvals = v["approvals"].as_array()?;
                    let first = approvals.first()?;
                    Some(first["request_id"].as_str()?.to_string())
                })
            }
        },
    )
    .await;
    // wire 契约（session_backend.rs + frontend client.ts 562-571）：内嵌标签
    // 枚举再套一层 `decision` 键——`{decision: {decision: "allow_once"}}`。
    let (ans_status, ans_body) = post_json(
        &cli,
        &format!("{}/api/permissions/{}/answer", sb.webui_url(), request_id),
        serde_json::json!({ "decision": { "decision": "allow_once" } }),
    )
    .await
    .expect("answer parked approval");
    assert_eq!(ans_status, 200, "answer must be delivered: {ans_body}");

    wait_turn_done(&cli, &sb, &detail_url).await;

    // 投影序列：tool_use（📖 Bash + 参数）→ 工具执行（✓ Bash + loop done）
    // → 环后正文，条目顺序单调。
    let (_status, detail) = get_json_status(&cli, &detail_url)
        .await
        .expect("session detail");
    let entries = detail["entries"].as_array().cloned().unwrap_or_default();
    let tool_start = entries.iter().position(|e| {
        e["element_type"].as_str() == Some("tool")
            && e["content"]
                .as_str()
                .is_some_and(|c| c.contains("📖 **Bash**") && c.contains("echo loop"))
    })
    .expect("tool_use entry (📖 Bash) must be in the projection");
    let tool_end = entries
        .iter()
        .position(|e| {
            e["element_type"].as_str() == Some("tool")
                && e["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("✓ **Bash**") && c.contains("loop done"))
        })
        .expect("tool execution entry (✓ Bash + loop done) must follow");
    let post_text = entries
        .iter()
        .position(|e| e["element_type"].as_str() == Some("markdown") && e["content"].as_str() == Some("tool loop finished"))
        .expect("post-tool text entry must close the loop");
    assert!(
        tool_start < tool_end && tool_end < post_text,
        "projection must read tool_use → execution → post-tool text in order"
    );

    // journal 逐行携带场景名（D4）：桩进程写下的每一行都点名 tool-loop。
    let lines = read_jsonl(&journal);
    assert!(
        !lines.is_empty(),
        "stub journal must have lines: {}",
        journal.display()
    );
    assert!(
        lines
            .iter()
            .all(|l| l["scenario"].as_str() == Some("tool-loop")),
        "every journal line must name the scenario: {lines:?}"
    );
}

/// spec 场景「空响应场景」前半句（close-acceptance-blind-spots 1.3）：empty
/// 桩正常结束回合、零输出——回合 Done 且投影无任何正文条目（只有操作者
/// prompt）。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn empty_scenario_ends_done_without_content_entries() {
    let sb = Sandbox::new("testsuite_e2e", "fake-empty");
    sb.append_acp_args(&["--scenario", "empty"]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "say nothing", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());

    wait_turn_done(&cli, &sb, &detail_url).await;

    let (_status, detail) = get_json_status(&cli, &detail_url)
        .await
        .expect("session detail");
    let entries = detail["entries"].as_array().cloned().unwrap_or_default();
    let contents: Vec<_> = entries
        .iter()
        .filter(|e| e["kind"].as_str() == Some("content"))
        .collect();
    assert!(
        contents.is_empty(),
        "a zero-output turn must project no content entries (prompt only): {contents:?}"
    );
    assert_eq!(
        detail["status_slug"].as_str(),
        Some("done"),
        "the empty turn must still end Done: {detail}"
    );
    // TODO(close-acceptance-blind-spots 组4)：空回合检测落地后，投影会多一条
    // 零输出合成提示（element_type == "notice"，spec「空响应场景」THEN 下半
    // 句）——届时把上面的「无 content 条目」改为「恰好一条 notice 条目」。
}

/// 慢响应触发停滞自愈（close-acceptance-blind-spots 1.3）：slow 桩先静默
/// `--delay-ms` 毫秒（静默期照常应答驱动探针——活着但对引擎沉默），delay
/// 大于 `[dispatch] turn_stall_timeout` 时看门狗强制收尾：投影出现「回合停滞
/// 被强制收尾」error 条目、会话到 Done，core.log 留 TurnStalled warn。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn slow_scenario_delay_trips_the_stall_watchdog() {
    let sb = Sandbox::new("testsuite_e2e", "fake-slow-stall");
    sb.set_turn_stall_timeout(2);
    sb.append_acp_args(&["--scenario", "slow", "--delay-ms", "9000"]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "take your time", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();

    // 看门狗收尾：transcript 就地点名收尾原因，会话终态 Done。
    wait_for(
        "watchdog to settle the slow turn",
        Duration::from_secs(30),
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
                    let settled = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| {
                            e["element_type"].as_str() == Some("error")
                                && e["content"]
                                    .as_str()
                                    .is_some_and(|c| c.contains("回合停滞被强制收尾"))
                        })
                    });
                    let done = v["status_slug"].as_str() == Some("done");
                    (settled && done).then_some(v)
                })
            }
        },
    )
    .await;

    // 通知事实可达：core.log 的 TurnStalled warn（与 stalled_turn 用例同源）。
    wait_log_contains(
        "core.log to carry the stall warning",
        &sb.core_log,
        &["turn stalled"],
    )
    .await;
}

/// 错误响应场景（close-acceptance-blind-spots 1.3）：error 桩以 is_error
/// result 帧模拟上游错误后退出——投影出现点名错误文案的 error 条目，会话
/// 落终态（failed；映射随会话死亡 retired 后即 dormant），绝不静默。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn error_scenario_projects_an_error_entry() {
    let sb = Sandbox::new("testsuite_e2e", "fake-error");
    sb.append_acp_args(&["--scenario", "error"]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "explode please", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();

    wait_for(
        "the upstream error entry to land in the projection",
        Duration::from_secs(30),
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
                    let errored = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| {
                            e["element_type"].as_str() == Some("error")
                                && e["content"]
                                    .as_str()
                                    .is_some_and(|c| c.contains("upstream error (fake)"))
                        })
                    });
                    // 终态：映射收尾前瞬间是 failed，随会话死亡 retired 成
                    // dormant（两个词都不是 working）——都在场才断言通过。
                    let terminal = matches!(
                        v["status_slug"].as_str(),
                        Some("failed") | Some("dormant")
                    );
                    (errored && terminal).then_some(v)
                })
            }
        },
    )
    .await;
}

/// 场景参数错误路径（close-acceptance-blind-spots 1.4 的进程级半边）：未知
/// `--scenario` 值在桩解析期 exit(2)——子进程秒退，driver/manager 以终态
/// 错误如实上报（投影 error 条目、会话 failed），绝不停在「握手成功后永远
/// 沉默」的假绿形态。
#[tokio::test]
#[ignore = "process-level e2e; run with -- --ignored or invoke testsuite-e2e"]
async fn unknown_scenario_arg_fails_the_turn_loudly() {
    let sb = Sandbox::new("testsuite_e2e", "fake-unknown-scenario");
    sb.append_acp_args(&["--scenario", "no-such-scenario"]);
    let cli = http_client();
    let _core = sb.spawn_core();
    let _webui = sb.spawn_webui(&sb.core_secret);
    wait_reachable(&cli, &sb).await;

    let project_id = scene_project_id(&cli, &sb).await;
    let (status, body) = post_json(
        &cli,
        &format!("{}/api/sessions", sb.webui_url()),
        serde_json::json!({ "project_id": project_id.clone(), "prompt": "hello", "agent": "claude" }),
    )
    .await
    .expect("create session");
    assert_eq!(status, 201, "create session: {body}");
    let key = body["key"].as_str().expect("key").to_string();
    let detail_url = format!("{}/api/sessions/{key}", sb.webui_url());
    let hint = sb.path.clone();

    wait_for(
        "the dead-stub turn to surface a terminal error",
        Duration::from_secs(45),
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
                    let errored = v["entries"].as_array().is_some_and(|entries| {
                        entries.iter().any(|e| e["element_type"].as_str() == Some("error"))
                    });
                    let failed = v["status_slug"].as_str() == Some("failed");
                    (errored && failed).then_some(v)
                })
            }
        },
    )
    .await;
}
