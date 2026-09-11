//! 端到端链路测试：**真节点**（`sebas_node`）与**真控制面**（`crate::node_link`）
//! 在同一个进程里通过真 websocket 对话（add-remote-execution-node group 3/4/5）。
//!
//! 这里验证的是两侧实现拼起来是否真的成立——单侧各自对着「假对手」的测试无法覆盖
//! 协议形状、配对流程、宿主行为与连接句柄之间的接缝：
//!
//! 1. 配对（join token → 长期凭据）后握手接入；
//! 2. `Spawn` / `Prompt` / `LogFrom` / `Snapshot` / `Close` 的请求-应答；
//! 3. turn 批**主动上行**（事件流）与日志**按需回拉**两条路都通；
//! 4. 关闭后日志仍可读（执行事实不因关闭消失）；
//! 5. 节点的**能力清单**如实上报（只有 echo，不编造）。

use sebas::node_link::placement::{self, PlacementError, ProjectRef};
use sebas::node_link::projection::RemoteProjection;
use sebas::node_link::{NodeLinkServer, RemoteFleet, RemoteSession, SessionLifecycle};
use sebas_node_link::{ApprovalDecision, SessionEvent, SessionOp, SessionResult};
use std::sync::Arc;
use std::time::Duration;

/// 起控制面 + 接入一个真节点，返回（沙箱目录, 服务端, 节点连接句柄）。
async fn paired_node() -> (
    tempfile::TempDir,
    Arc<NodeLinkServer>,
    Arc<sebas::node_link::NodeConnection>,
) {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(
        NodeLinkServer::bind("127.0.0.1:0", dir.path().join("nodes.json"))
            .await
            .unwrap(),
    );
    let token = server
        .registry()
        .lock()
        .await
        .issue_join_token(sebas::node_link::server::now_unix(), 600)
        .unwrap();
    let url = format!("ws://{}", server.local_addr().unwrap());

    let serving = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = serving.accept_one().await;
    });

    // 真节点：用一次性 join token 配对并进入链路循环。
    let store = sebas_node::IdentityStore::new(dir.path().join("node-state"));
    let client = sebas_node::link::LinkClient::new(
        url,
        store.load_or_create_id(Some("itest-node")).unwrap(),
        store,
        Some(token),
        dir.path().join("node-sessions"),
        dir.path().join("node-materials"),
        4,
    );
    tokio::spawn(async move {
        let _ = std::sync::Arc::new(client).run().await;
    });

    let conn = loop {
        if let Some(c) = server.live_connection("itest-node").await {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    (dir, server, conn)
}

#[tokio::test]
async fn a_real_node_hosts_a_session_and_streams_turns_over_the_link() {
    let (_tmp, server, conn) = paired_node().await;

    // 事件订阅要在动作之前建立，否则会漏掉主动上行的批。
    let mut events = conn.subscribe();

    // 1) 建立会话：节点如实回报实际生效的执行体（echo，不是模型）。
    match conn
        .request(SessionOp::Spawn {
            session_id: "s-1".into(),
            project_dir: None,
            agent_kind: Some("echo".into()),
            model: None,
            mode: Some("ask".into()),
            provider: None,
        })
        .await
        .unwrap()
    {
        SessionResult::Spawned {
            agent_kind, mode, ..
        } => {
            assert_eq!(agent_kind, "echo");
            assert_eq!(mode.as_deref(), Some("ask"), "mode 如实回报");
        }
        other => panic!("应建立会话，实际 {other:?}"),
    }

    // 2) 投递输入：节点接受。
    assert_eq!(
        conn.request(SessionOp::Prompt {
            session_id: "s-1".into(),
            text: "hello".into(),
        })
        .await
        .unwrap(),
        SessionResult::Ok
    );

    // 3) 主动上行：等到包含 echo 输出的 turn 批。
    let mut saw_output = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !saw_output {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(SessionEvent::TurnBatch { entries, .. })) => {
                if entries.iter().any(|e| e.text == "echo: hello") {
                    saw_output = true;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => panic!("事件流失败：{e}"),
            Err(_) => break,
        }
    }
    assert!(saw_output, "应收到含 echo 输出的 turn 批");

    // 4) 按需回拉：拿到精确序列（传输合并不影响保真）。
    let mut view = RemoteSession::new("s-1");
    view.reconcile(&conn).await.unwrap();
    assert!(
        view.entries().iter().any(|e| e.text == "echo: hello"),
        "回拉应包含 echo 输出：{:?}",
        view.entries()
    );
    let cursor_after_prompt = view.cursor();
    assert!(cursor_after_prompt >= 4, "spawning/active/prompt/output");

    // 5) 快照：状态=幂等快照，重复应用结果不变。
    match conn
        .request(SessionOp::Snapshot {
            session_id: "s-1".into(),
        })
        .await
        .unwrap()
    {
        SessionResult::Snapshot {
            summary,
            last_seq,
            reclaimed_through_seq,
            ..
        } => {
            assert_eq!(summary.session_id, "s-1");
            assert_eq!(summary.phase, "active");
            assert!(last_seq >= cursor_after_prompt);
            assert_eq!(reclaimed_through_seq, 0, "尚未回收");
            view.note_snapshot(&summary, reclaimed_through_seq);
        }
        other => panic!("应给出快照，实际 {other:?}"),
    }

    // 6) 关闭后**日志仍可读**：执行事实不因关闭消失（否则最后几轮可能永远丢）。
    assert_eq!(
        conn.request(SessionOp::Close {
            session_id: "s-1".into(),
        })
        .await
        .unwrap(),
        SessionResult::Ok
    );
    match conn
        .request(SessionOp::LogFrom {
            session_id: "s-1".into(),
            from_seq: 1,
        })
        .await
        .unwrap()
    {
        SessionResult::Log { entries, .. } => {
            assert!(
                entries.iter().any(|e| e.text == "closed"),
                "关闭应留下痕迹：{entries:?}"
            );
            assert!(entries.iter().any(|e| e.text == "echo: hello"));
        }
        other => panic!("关闭后回拉应成功，实际 {other:?}"),
    }

    // 7) 关闭后不再接受输入，且是**可判别**的拒绝（不是断链）。
    let rejected = conn
        .request(SessionOp::Prompt {
            session_id: "s-1".into(),
            text: "again".into(),
        })
        .await
        .unwrap();
    let (code, cause) = sebas_node_link::session_rejection_of(&rejected).unwrap();
    assert_eq!(code, sebas_node_link::SessionRejectCode::SessionClosed);
    assert!(cause.contains("已关闭"), "{cause}");

    // 8) 未知会话也是可判别拒绝。
    let rejected = conn
        .request(SessionOp::Cancel {
            session_id: "ghost".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        sebas_node_link::session_rejection_of(&rejected).unwrap().0,
        sebas_node_link::SessionRejectCode::UnknownSession
    );

    // 撤回后服务端应把节点标离线。
    let _ = server;
}

#[tokio::test]
async fn spawn_rejects_an_unusable_project_dir_with_a_named_cause() {
    let (tmp, _server, conn) = paired_node().await;
    let missing = tmp.path().join("not-a-repo");

    let rejected = conn
        .request(SessionOp::Spawn {
            session_id: "s-2".into(),
            project_dir: Some(missing.to_string_lossy().into_owned()),
            agent_kind: Some("echo".into()),
            model: None,
            mode: None,
            provider: None,
        })
        .await
        .unwrap();
    let (code, cause) = sebas_node_link::session_rejection_of(&rejected).unwrap();
    assert_eq!(code, sebas_node_link::SessionRejectCode::UnusableProjectDir);
    assert!(cause.contains("not-a-repo"), "成因要指名路径：{cause}");

    // 存在的目录可用。
    assert!(matches!(
        conn.request(SessionOp::Spawn {
            session_id: "s-3".into(),
            project_dir: Some(tmp.path().to_string_lossy().into_owned()),
            agent_kind: Some("echo".into()),
            model: None,
            mode: None,
            provider: None,
        })
        .await
        .unwrap(),
        SessionResult::Spawned { .. }
    ));
}

#[tokio::test]
async fn an_unconfigured_agent_kind_is_refused_honestly() {
    let (_tmp, _server, conn) = paired_node().await;
    let rejected = conn
        .request(SessionOp::Spawn {
            session_id: "s-4".into(),
            project_dir: None,
            agent_kind: Some("claude".into()), // 节点上没配
            model: None,
            mode: None,
            provider: None,
        })
        .await
        .unwrap();
    let (code, cause) = sebas_node_link::session_rejection_of(&rejected).unwrap();
    assert_eq!(
        code,
        sebas_node_link::SessionRejectCode::UnsupportedAgentKind
    );
    assert!(cause.contains("claude"), "{cause}");
}


// ── 放置（3.1 / 3.4）对真链路 ────────────────────────────────────────────────

#[tokio::test]
async fn placement_uses_the_projects_node_and_namespaces_the_session_id() {
    let (tmp, server, _conn) = paired_node().await;
    let project = ProjectRef {
        id: "proj-a".into(),
        node_id: "itest-node".into(),
        path: tmp.path().to_string_lossy().into_owned(),
    };

    let placed = placement::place_and_spawn(
        &server,
        Some(&project),
        Some("some-other-node"), // 有项目时默认节点不参与
        Some("echo"),
        Some("m1"),
        Some("ask"),
    )
    .await
    .unwrap();

    assert_eq!(placed.placement.node_id, "itest-node");
    assert_eq!(placed.placement.project_dir.as_deref(), Some(project.path.as_str()));
    assert_eq!(
        placed.placement.session_id.namespace(),
        "proj-a",
        "会话 id 按项目命名空间发行"
    );
    assert_eq!(placed.agent_kind, "echo");
    assert_eq!(placed.model.as_deref(), Some("m1"));
    assert_eq!(placed.mode.as_deref(), Some("ask"), "实际生效值如实回报");
}

#[tokio::test]
async fn placement_falls_back_to_the_default_node_for_project_less_sessions() {
    let (_tmp, server, _conn) = paired_node().await;
    let placed = placement::place_and_spawn(
        &server,
        None,
        Some("itest-node"),
        Some("echo"),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(placed.placement.node_id, "itest-node");
    assert_eq!(placed.placement.project_dir, None);
    assert_eq!(placed.placement.session_id.namespace(), "(no-project)");
}

#[tokio::test]
async fn placement_refuses_an_offline_node_without_creating_a_placeholder() {
    let (_tmp, server, conn) = paired_node().await;
    let project = ProjectRef {
        id: "proj-b".into(),
        node_id: "ghost-node".into(), // 没有这台节点在线
        path: "/srv/whatever".into(),
    };

    match placement::place_and_spawn(&server, Some(&project), None, Some("echo"), None, None).await {
        Err(PlacementError::NodeOffline { node_id }) => assert_eq!(node_id, "ghost-node"),
        other => panic!("应如实报节点离线，实际 {other:?}"),
    }

    // **不建占位会话**：真节点上一个会话都没有。
    match conn.request(SessionOp::ListSessions).await.unwrap() {
        SessionResult::Sessions { sessions } => {
            assert!(sessions.is_empty(), "离线失败不应在任何节点上留下占位：{sessions:?}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn placement_needs_a_project_or_a_configured_default_node() {
    let (_tmp, server, _conn) = paired_node().await;
    match placement::place_and_spawn(&server, None, None, Some("echo"), None, None).await {
        Err(PlacementError::NoProjectAndNoDefaultNode) => {}
        other => panic!("无项目且无默认节点应如实失败，实际 {other:?}"),
    }
}

#[tokio::test]
async fn placement_maps_a_node_rejection_and_names_the_node() {
    let (tmp, server, _conn) = paired_node().await;
    let missing = tmp.path().join("no-such-repo");
    let project = ProjectRef {
        id: "proj-c".into(),
        node_id: "itest-node".into(),
        path: missing.to_string_lossy().into_owned(),
    };

    match placement::place_and_spawn(&server, Some(&project), None, Some("echo"), None, None).await {
        Err(PlacementError::Rejected { node_id, code, cause }) => {
            assert_eq!(node_id, "itest-node", "失败要指名节点");
            assert_eq!(code, sebas_node_link::SessionRejectCode::UnusableProjectDir);
            assert!(cause.contains("no-such-repo"), "{cause}");
        }
        other => panic!("应给出带节点名的拒绝，实际 {other:?}"),
    }
}


// ── 审批走廊（6.2 / 6.4 / 6.5 / 6.6 / 6.7）对真链路 ──────────────────────────

async fn spawn_session(
    conn: &sebas::node_link::NodeConnection,
    session_id: &str,
    mode: Option<&str>,
) -> SessionResult {
    conn.request(SessionOp::Spawn {
        session_id: session_id.into(),
        project_dir: None,
        agent_kind: Some("echo".into()),
        model: None,
        mode: mode.map(str::to_string),
        provider: None,
    })
    .await
    .unwrap()
}

/// 等一个满足谓词的事件（超时即失败）。
async fn wait_event<F>(
    events: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
    mut predicate: F,
) -> SessionEvent
where
    F: FnMut(&SessionEvent) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(event)) => {
                if predicate(&event) {
                    return event;
                }
            }
            Ok(Err(e)) => panic!("事件流失败：{e}"),
            Err(_) => break,
        }
    }
    panic!("等不到预期事件");
}

#[tokio::test]
async fn ask_mode_raises_an_approval_request_that_only_the_control_plane_can_resolve() {
    let (_tmp, _server, conn) = paired_node().await;
    let mut events = conn.subscribe();
    spawn_session(&conn, "s-ask", Some("ask")).await;

    // 受门控动作：节点停驻并上报。
    conn.request(SessionOp::Prompt {
        session_id: "s-ask".into(),
        text: "run: ls -la".into(),
    })
    .await
    .unwrap();

    let request_id = match wait_event(&mut events, |e| {
        matches!(e, SessionEvent::ApprovalRequested { .. })
    })
    .await
    {
        SessionEvent::ApprovalRequested { request_id, tool, .. } => {
            assert_eq!(tool, "bash");
            request_id
        }
        other => panic!("{other:?}"),
    };

    // 对账：悬空请求可见（主控停机期间 park 的，回来就能看见）。
    let mut view = RemoteSession::new("s-ask");
    assert_eq!(view.reconcile_approvals(&conn).await.unwrap(), 1);
    assert_eq!(view.parked_approvals()[0].request_id, request_id);

    // 只有控制面能决议。
    assert!(view
        .answer_approval(&conn, &request_id, ApprovalDecision::AllowOnce)
        .await
        .unwrap());
    assert_eq!(view.reconcile_approvals(&conn).await.unwrap(), 0, "决议后不再悬空");

    // 审计落到了节点日志：谁给的结论、什么结论、针对哪个请求。
    match conn
        .request(SessionOp::LogFrom {
            session_id: "s-ask".into(),
            from_seq: 1,
        })
        .await
        .unwrap()
    {
        SessionResult::Log { entries, .. } => {
            let audit = entries
                .iter()
                .find(|e| e.kind == "audit" && e.text.contains("allow_once"))
                .expect("决议必须留审计");
            assert!(audit.text.contains(&request_id), "{}", audit.text);
            // 未获批准前不得有执行痕迹。
            assert!(
                !entries.iter().any(|e| e.text.contains("ls -la") && e.kind == "output"),
                "停驻期间不得执行：{entries:?}"
            );
        }
        other => panic!("{other:?}"),
    }

    // 迟到的重复决定：可判别拒绝。
    let rejected = conn
        .request(SessionOp::ApprovalAnswer {
            session_id: "s-ask".into(),
            request_id: request_id.clone(),
            decision: ApprovalDecision::AllowOnce,
        })
        .await
        .unwrap();
    assert_eq!(
        sebas_node_link::session_rejection_of(&rejected).unwrap().0,
        sebas_node_link::SessionRejectCode::UnknownApprovalRequest
    );
}

#[tokio::test]
async fn auto_mode_never_raises_requests_and_says_who_allowed_it() {
    let (_tmp, _server, conn) = paired_node().await;
    let mut events = conn.subscribe();
    spawn_session(&conn, "s-auto", Some("auto")).await;

    conn.request(SessionOp::Prompt {
        session_id: "s-auto".into(),
        text: "run: rm -rf /tmp/nothing".into(),
    })
    .await
    .unwrap();

    match wait_event(&mut events, |e| {
        matches!(e, SessionEvent::GateResolved { .. })
    })
    .await
    {
        SessionEvent::GateResolved {
            decision, source, ..
        } => {
            assert_eq!(decision, "auto_allowed");
            assert_eq!(source, "mode:auto", "自动放行必须标明来源");
        }
        other => panic!("{other:?}"),
    }

    // 一条请求都没产生。
    let mut view = RemoteSession::new("s-auto");
    assert_eq!(view.reconcile_approvals(&conn).await.unwrap(), 0);
}

#[tokio::test]
async fn switching_to_auto_is_audited_and_stops_the_requests() {
    let (_tmp, _server, conn) = paired_node().await;
    spawn_session(&conn, "s-switch", Some("ask")).await;

    match conn
        .request(SessionOp::SetMode {
            session_id: "s-switch".into(),
            mode: "auto".into(),
        })
        .await
        .unwrap()
    {
        SessionResult::ModelSet { model } => {
            assert_eq!(model.as_deref(), Some("auto"), "回报实际生效模式")
        }
        other => panic!("{other:?}"),
    }

    // 之后受门控动作不再停驻。
    conn.request(SessionOp::Prompt {
        session_id: "s-switch".into(),
        text: "run: ls".into(),
    })
    .await
    .unwrap();
    let mut view = RemoteSession::new("s-switch");
    assert_eq!(view.reconcile_approvals(&conn).await.unwrap(), 0);

    // 审计里能查到 auto 是控制面开的。
    match conn
        .request(SessionOp::LogFrom {
            session_id: "s-switch".into(),
            from_seq: 1,
        })
        .await
        .unwrap()
    {
        SessionResult::Log { entries, .. } => assert!(
            entries
                .iter()
                .any(|e| e.kind == "audit" && e.text.contains("mode=auto")),
            "开启 auto 必须留审计：{entries:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_mode_is_refused_without_downgrading() {
    let (_tmp, _server, conn) = paired_node().await;
    let rejected = conn
        .request(SessionOp::Spawn {
            session_id: "s-bad".into(),
            project_dir: None,
            agent_kind: Some("echo".into()),
            model: None,
            mode: Some("yolo".into()),
            provider: None,
        })
        .await
        .unwrap();
    let (code, cause) = sebas_node_link::session_rejection_of(&rejected).unwrap();
    assert_eq!(code, sebas_node_link::SessionRejectCode::UnsupportedMode);
    assert!(cause.contains("yolo"), "{cause}");
}


// ── 会话寿命绑定节点（5.6）────────────────────────────────────────────────────

/// 可控的两进程装配：能停节点、能用同一/不同状态目录重启节点。
struct Harness {
    dir: tempfile::TempDir,
    server: Arc<NodeLinkServer>,
    url: String,
    token: Option<String>,
    node_task: Option<tokio::task::JoinHandle<()>>,
    /// 持有发送端：drop 即让服务循环退出。
    _shutdown: tokio::sync::watch::Sender<bool>,
    /// 本次起节点要上报的能力清单（缺省空清单 = 如实表示什么都没配）。
    manifest: sebas_node_link::CapabilityManifest,
    /// 控制面材料仓（7.3/7.4）。
    materials: std::sync::Arc<sebas::node_link::MaterialStore>,
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let materials = sebas::node_link::MaterialStore::new();
        let server = Arc::new(
            NodeLinkServer::bind("127.0.0.1:0", dir.path().join("nodes.json"))
                .await
                .unwrap()
                .with_inbound_handler(
                    std::sync::Arc::clone(&materials)
                        as std::sync::Arc<dyn sebas::node_link::client::InboundHandler>,
                ),
        );
        let token = server
            .registry()
            .lock()
            .await
            .issue_join_token(sebas::node_link::server::now_unix(), 600)
            .unwrap();
        let url = format!("ws://{}", server.local_addr().unwrap());
        // 长驻服务循环：这个 harness 需要**多次**接入（停一个、起一个）。
        let (shutdown, rx) = tokio::sync::watch::channel(false);
        let serving = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = serving.serve(rx).await;
        });
        Self {
            dir,
            server,
            url,
            token: Some(token),
            node_task: None,
            _shutdown: shutdown,
            manifest: sebas_node_link::CapabilityManifest::default(),
            materials,
        }
    }

    fn node_state_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("node-state")
    }

    /// 会话目录跟状态目录走（生产里就是 `state_dir/sessions`）：
    /// **同一状态目录重启 → 日志还在**；换新状态目录（重装）→ 什么都不剩。
    fn sessions_dir_for(state_dir: &std::path::Path) -> std::path::PathBuf {
        state_dir.join("sessions")
    }

    /// 用给定状态目录起一个节点进程并等它接入（返回控制面侧句柄）。
    async fn start_node(&mut self, state_dir: &std::path::Path) -> Arc<sebas::node_link::NodeConnection> {
        let store = sebas_node::IdentityStore::new(state_dir.to_path_buf());
        let client = sebas_node::link::LinkClient::new(
            self.url.clone(),
            store.load_or_create_id(Some("itest-node")).unwrap(),
            store,
            self.token.take(),
            Self::sessions_dir_for(state_dir),
            state_dir.join("materials"),
            4,
        )
        .with_manifest(self.manifest.clone());
        let client = std::sync::Arc::new(client);
        self.node_task = Some(tokio::spawn(async move {
            let _ = client.run().await;
        }));
        // 有界等待：起不来时给出清晰失败，而不是把测试挂死。
        for _ in 0..500 {
            if let Some(c) = self.server.live_connection("itest-node").await {
                return c;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("节点未在 10 秒内接入（配对被拒？端口占用？）");
    }

    /// 停掉节点（abort 任务 = 进程消失），并等控制面把它标离线。
    async fn stop_node(&mut self) {
        if let Some(task) = self.node_task.take() {
            task.abort();
        }
        for _ in 0..200 {
            if self.server.live_connection("itest-node").await.is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("节点未在预期时间内下线");
    }
}

#[tokio::test]
async fn a_control_plane_rebuild_resumes_sessions_instead_of_recreating_them() {
    // 「杀主控」：主控重启不终止远端执行，而且**不重建**会话。
    let mut h = Harness::start().await;
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let placed = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-a")),
        None,
        Some("echo"),
        None,
        Some("ask"),
    )
    .await
    .unwrap();
    let session_id = placed.placement.session_id.as_str().to_string();
    conn.request(SessionOp::Prompt {
        session_id: session_id.clone(),
        text: "hello".into(),
    })
    .await
    .unwrap();

    // 主控侧视图整个丢掉再重建（模拟主控进程重启）。
    let mut fleet = RemoteFleet::new();
    assert!(fleet.view(&session_id).is_none());

    let adopted = fleet.adopt_from_node(&conn).await.unwrap();
    assert_eq!(adopted, vec![session_id.clone()], "从节点认领回身份");
    assert!(
        matches!(fleet.lifecycle(&session_id), Some(SessionLifecycle::Live { .. })),
        "重建后仍是在线会话，不是终止"
    );

    // 日志照样拉得到（执行事实在节点上）。
    fleet
        .view_mut(&session_id)
        .unwrap()
        .reconcile(&conn)
        .await
        .unwrap();
    assert!(
        fleet
            .view(&session_id)
            .unwrap()
            .entries()
            .iter()
            .any(|e| e.text == "echo: hello"),
        "重建后仍能拉回历史"
    );

    // **没有重建**：节点上仍然只有这一个会话。
    match conn.request(SessionOp::ListSessions).await.unwrap() {
        SessionResult::Sessions { sessions } => assert_eq!(sessions.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_link_loss_marks_sessions_offline_and_a_restart_terminates_them_with_the_logs_intact() {
    let mut h = Harness::start().await;
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let placed = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-b")),
        None,
        Some("echo"),
        None,
        Some("ask"),
    )
    .await
    .unwrap();
    let session_id = placed.placement.session_id.as_str().to_string();
    conn.request(SessionOp::Prompt {
        session_id: session_id.clone(),
        text: "before-restart".into(),
    })
    .await
    .unwrap();

    let mut fleet = RemoteFleet::new();
    fleet.track("itest-node", &session_id);

    // ① 链路断了：**不终止**，只是暂时离线。
    h.stop_node().await;
    assert_eq!(fleet.on_node_disconnected("itest-node"), 1);
    assert!(
        fleet.lifecycle(&session_id).unwrap().is_alive(),
        "链路断开不得终止会话"
    );

    // ② 节点用**同一状态目录**重启（凭据仍在，无需新 token）。
    let conn2 = h.start_node(&state).await;
    let report = fleet.reconcile_node(&conn2).await.unwrap();
    assert_eq!(report.terminated.len(), 1, "节点重启即终止其会话");
    let (id, cause) = &report.terminated[0];
    assert_eq!(id, &session_id);
    assert!(cause.contains("terminated"), "成因来自节点：{cause}");
    assert!(matches!(
        fleet.lifecycle(&session_id),
        Some(SessionLifecycle::Terminated { .. })
    ));

    // ③ 对账**不重建**：节点上还是那一个（且相位是 terminated）。
    match conn2.request(SessionOp::ListSessions).await.unwrap() {
        SessionResult::Sessions { sessions } => {
            assert_eq!(sessions.len(), 1, "对账不得新建会话");
            assert_eq!(sessions[0].phase, "terminated");
        }
        other => panic!("{other:?}"),
    }

    // ④ 但**执行事实没丢**：重启后日志仍可拉回（这正是宿主挂回孤立日志的意义）。
    match conn2
        .request(SessionOp::LogFrom {
            session_id: session_id.clone(),
            from_seq: 1,
        })
        .await
        .unwrap()
    {
        SessionResult::Log { entries, .. } => assert!(
            entries.iter().any(|e| e.text == "before-restart"
                || e.text == "echo: before-restart"),
            "重启后仍应能拉回重启前的条目：{entries:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_reinstalled_node_makes_its_sessions_read_as_gone() {
    let mut h = Harness::start().await;
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let placed = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-c")),
        None,
        Some("echo"),
        None,
        None,
    )
    .await
    .unwrap();
    let session_id = placed.placement.session_id.as_str().to_string();

    let mut fleet = RemoteFleet::new();
    fleet.track("itest-node", &session_id);

    // 换一台**全新状态目录**的机器（重装）：需要重新配对。
    h.stop_node().await;
    fleet.on_node_disconnected("itest-node");
    h.token = Some(
        h.server
            .registry()
            .lock()
            .await
            .issue_join_token(sebas::node_link::server::now_unix(), 600)
            .unwrap(),
    );
    let fresh_state = h.dir.path().join("fresh-node-state");
    let conn2 = h.start_node(&fresh_state).await;

    let report = fleet.reconcile_node(&conn2).await.unwrap();
    assert_eq!(report.terminated.len(), 1);
    assert!(
        report.terminated[0].1.contains("已不存在"),
        "重装后节点不认识该会话：{}",
        report.terminated[0].1
    );
    // 不重建。
    match conn2.request(SessionOp::ListSessions).await.unwrap() {
        SessionResult::Sessions { sessions } => assert!(sessions.is_empty()),
        other => panic!("{other:?}"),
    }
}


#[tokio::test]
async fn the_control_plane_sees_the_nodes_capability_manifest() {
    let mut h = Harness::start().await;
    h.manifest = sebas_node_link::CapabilityManifest {
        agent_kinds: vec![
            sebas_node_link::AgentKindCapability {
                kind: "echo".into(),
                reachable: true,
                cause: None,
            },
            sebas_node_link::AgentKindCapability {
                kind: "claude".into(),
                reachable: false,
                cause: Some("命令 \"claude\" 不在 PATH 上".into()),
            },
        ],
        providers: vec!["anthropic".into()],
        mode_enforcement: vec![sebas_node_link::ModeEnforcement {
            execution_body: "echo".into(),
            enforces_mode: true,
        }],
    };
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let manifest = conn.manifest();
    assert!(
        manifest
            .agent_kinds
            .iter()
            .any(|k| k.kind == "echo" && k.reachable),
        "可达 kind 应上报"
    );
    assert!(
        manifest
            .agent_kinds
            .iter()
            .any(|k| k.kind == "claude" && !k.reachable && k.cause.is_some()),
        "不可达 kind 必须带成因，控制面才能只提供可达项"
    );
    assert_eq!(manifest.providers, vec!["anthropic".to_string()]);
    assert!(
        manifest
            .mode_enforcement
            .iter()
            .any(|e| e.execution_body == "echo" && e.enforces_mode)
    );
}


// ── 材料过河（7.3 / 7.4 / 7.5）──────────────────────────────────────────────

fn material(path: &str, content: &str) -> sebas_node_link::MaterialFile {
    sebas_node_link::MaterialFile {
        path: path.into(),
        content: content.into(),
    }
}

#[tokio::test]
async fn the_node_pulls_operator_materials_at_spawn_and_pins_the_version() {
    let mut h = Harness::start().await;
    h.materials
        .set_bundle(
            "v1",
            vec![
                material("skills/beads/SKILL.md", "# beads v1"),
                material("memory/notes.md", "remember this"),
            ],
        )
        .unwrap();
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let placed = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-m")),
        None,
        Some("echo"),
        None,
        Some("ask"),
    )
    .await
    .unwrap();
    assert_eq!(
        placed.materials_version.as_deref(),
        Some("v1"),
        "会话应钉住本次拉取到的版本"
    );

    // 材料真的落在**执行体认得的位置**（echo 的落点约定）。
    let body_dir = state.join("materials/v1/echo");
    assert_eq!(
        std::fs::read_to_string(body_dir.join("skills/beads/SKILL.md")).unwrap(),
        "# beads v1"
    );
    assert_eq!(
        std::fs::read_to_string(body_dir.join("memory/notes.md")).unwrap(),
        "remember this"
    );

    // 日志留痕：事后要能回答"这个会话当时用的是哪一版"。
    match conn
        .request(SessionOp::LogFrom {
            session_id: placed.placement.session_id.as_str().to_string(),
            from_seq: 1,
        })
        .await
        .unwrap()
    {
        SessionResult::Log { entries, .. } => assert!(
            entries
                .iter()
                .any(|e| e.kind == "materials_pinned" && e.text.contains("v1")),
            "钉版本必须留痕：{entries:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_new_material_version_does_not_retroactively_change_existing_sessions() {
    let mut h = Harness::start().await;
    h.materials
        .set_bundle("v1", vec![material("a.md", "one")])
        .unwrap();
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;

    let first = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-m")),
        None,
        Some("echo"),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(first.materials_version.as_deref(), Some("v1"));

    // 控制面出新版本：**通知只是信号**（此处直接换仓模拟），节点不会自己去拉。
    h.materials
        .set_bundle("v2", vec![material("a.md", "two")])
        .unwrap();
    assert!(
        !state.join("materials/v2").exists(),
        "换版本本身不得让节点拉取——拉取只发生在会话创建时"
    );

    // 新会话拿到新版本…
    let second = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-m")),
        None,
        Some("echo"),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(second.materials_version.as_deref(), Some("v2"));
    assert_eq!(
        std::fs::read_to_string(state.join("materials/v2/echo/a.md")).unwrap(),
        "two"
    );

    // …而**旧会话仍然钉在 v1**（可复现性）。
    match conn
        .request(SessionOp::Snapshot {
            session_id: first.placement.session_id.as_str().to_string(),
        })
        .await
        .unwrap()
    {
        SessionResult::Snapshot { summary, .. } => {
            assert_eq!(summary.materials_version.as_deref(), Some("v1"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(state.join("materials/v1/echo/a.md")).unwrap(),
        "one",
        "旧版本内容不被覆盖"
    );
}

#[tokio::test]
async fn an_unconfigured_control_plane_leaves_the_session_without_materials() {
    let mut h = Harness::start().await;
    let state = h.node_state_dir();
    let conn = h.start_node(&state).await;
    // 没 set_bundle → 节点拉取会得到如实拒绝；会话照常建立，但不钉任何版本。
    let placed = sebas::node_link::placement::spawn_on(
        &conn,
        sebas::node_link::RemoteSessionId::issue(Some("proj-m")),
        None,
        Some("echo"),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(placed.materials_version, None, "没有材料就不假装钉了版本");
}

/// 主控重启后投影能否**从节点重建**（5.1/5.3 的关键性质）。
///
/// 进程级 e2e 抓到过一次失败：重建出来的空视图在回拉之前先被快照把游标推到了
/// 日志末尾，于是"对账完成"却一条转写都没有。这条测试把那段逻辑钉在毫秒级。
#[tokio::test]
async fn a_rebuilt_projection_recovers_the_transcript_from_the_node() {
    let (_dir, _server, conn) = paired_node().await;

    // 第一代控制面：在节点上建会话并跑一轮。
    let first = RemoteProjection::new();
    first.attach_connection(Arc::clone(&conn)).await;
    let (key, placed) = first
        .spawn_on("itest-node", None, Some("echo"), None, None, Some("hello"))
        .await
        .expect("在节点上建会话");
    let session_id = placed.placement.session_id.as_str().to_string();
    assert_eq!(key.reference, sebas::node_link::projection::row_reference("itest-node", &session_id));

    // 等节点把这一轮写完（事件流到达）。
    let mut got = false;
    for _ in 0..200 {
        if first
            .turns(&session_id, 0)
            .await
            .iter()
            .any(|e| e.content.contains("echo: hello"))
        {
            got = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(got, "第一代控制面应看到这一轮");

    // 第二代控制面：**全新的空投影**（= 主控进程重启），只拿节点的事实重建。
    let second = RemoteProjection::new();
    second.attach_connection(Arc::clone(&conn)).await;
    let report = second
        .observe_node(&conn)
        .await
        .expect("对账应当成功");
    assert_eq!(
        report.resumed.len(),
        1,
        "重建必须认领节点上的会话，而不是报 0 个"
    );

    let turns = second.turns(&session_id, 0).await;
    let text: String = turns.iter().map(|e| e.content.clone()).collect();
    assert!(
        text.contains("echo: hello"),
        "重建后的转写必须从节点补回来，实际：{text:?}（{turns:?}）"
    );
    // 行也要在：工作台据此显示这个会话。
    let rows = second.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id.as_deref(), Some(session_id.as_str()));
}
