//! feishu → 原生 sebas-agent 内核 → webui 呈现的端到端链路
//! （make-feishu-optional-webui-primary，design D2/D3；
//!  make-pi-the-builtin-agent 1.3 收口）。
//!
//! **本 change 后**：内置 `native` 槽位改指 pi（ACP 侧），飞书新会话不再进
//! 进程内内核——桥只服务 **legacy** `agent-*` 会话（既有内核会话的 resume/
//! 续聊读路径，design D5）。本文件证明该 legacy 路径仍完整可用：
//! 飞书 `agent-*` 文本经 router dispatch 路由到 `DispatchNativeBridge` →
//! 原生内核创建/续聊会话并登记 mapping → webui `InProcessBackend` snapshot
//! 看到该会话 → transcript（工具轨迹 + 收尾文本）可读 → 权限请求可经 webui
//! 审查卡回填。

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_webui::session_backend::{InProcessBackend, PermissionDecision, SessionBackend};
use std::sync::Arc;
use std::time::Duration;

/// 原生内核 manager（fake LLM：先工具调用触发权限，再收尾文本）。
fn native_manager() -> Arc<sebas_agent::session::SessionManager> {
    let llm = sebas_agent::llm::fake::FakeLlmClient::scripted(vec![
        sebas_agent::llm::fake::FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "rm -rf build"}),
        )]),
        sebas_agent::llm::fake::FakeLlmClient::say("native done"),
    ]);
    Arc::new(
        sebas_agent::session::SessionManager::new(
            Arc::new(llm),
            sebas_agent::tools::ToolRegistry::with_sandbox(
                Duration::from_secs(10),
                sebas_agent::policy::SandboxMode::Firewall,
            ),
            Default::default(),
        )
        .with_policy(Arc::new(sebas_agent::policy::PolicyEngine::new(
            Default::default(),
        )))
        .with_approver(sebas_agent::policy::ApproverHub::new()),
    )
}

/// legacy `agent-*` 会话（feishu 通道 + 裸 `agent-{hex}` 引用）仍是内核路径：
/// dispatch 走桥、webui 快照可见、transcript 可读、权限可回填。
#[tokio::test]
async fn legacy_native_session_appears_in_webui_snapshot() {
    let map = SessionMap::new();
    // legacy 形态：feishu 通道 + `agent-*` 引用（webui NativeAgentBackend 铸的
    // 历史内核 key）。
    let key = ChannelKey::new("feishu", "agent-legacy01");
    let (router, mut out_rx) = DispatchHandle::new(map);
    // 丢弃 Out（原生路径不发 SpawnAcp/卡片）。
    tokio::spawn(async move { while out_rx.recv().await.is_some() {} });

    // 装配真实桥（core 侧 = run.rs 的装配方式）；`native_default` 已退休，
    // legacy 路由由 key 前缀决定。
    let bridge = sebas::native_dispatch_bridge::DispatchNativeBridge::with_default(
        native_manager(),
        router.clone(),
        false,
    );
    router.set_native_bridge(Some(bridge)).await;

    let backend = InProcessBackend::new(router.clone());

    // 审查卡流：bash 属于默认政策 Ask（fail-closed），需要 webui 回填
    // allow-once 才能继续 turn——这同时验证权限呈现链路。必须先订阅再
    // dispatch（broadcast 只转发订阅后的事件）。
    let mut notices = backend.permission_requests().expect("backend has notices");

    // legacy feishu 文本 → dispatch → 走桥创建内核会话。
    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "build it".into(),
            reply_target: None,
        })
        .await;

    let notice = tokio::time::timeout(Duration::from_secs(5), notices.recv())
        .await
        .expect("permission notice should surface to webui")
        .expect("notice");
    assert_eq!(notice.tool_name, "bash");
    assert!(
        backend
            .answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
            .await,
        "webui answer should reach the native kernel approver"
    );

    // webui snapshot 应看到该会话（status active）。
    let row = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snap = backend.snapshot().await;
            if let Some(info) = snap
                .into_iter()
                .find(|i| i.channel == key.channel_str() && i.key == key.reference)
            {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("legacy native feishu session should appear in webui snapshot");
    assert_eq!(row.channel, "feishu");
    assert_eq!(row.key, "agent-legacy01");
    assert_eq!(row.status, "active".into());

    // transcript 可读：工具轨迹 + 收尾文本最终出现在 turn 流。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut last: Vec<sebas_dispatch::TurnEntry>;
    loop {
        last = backend.turns(key.clone(), 0).await.unwrap_or_default();
        let joined: String = last.iter().map(|e| e.content.clone()).collect();
        if joined.contains("bash") && joined.contains("native done") {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let joined: String = last.iter().map(|e| e.content.clone()).collect();
    assert!(
        joined.contains("bash"),
        "tool trace in transcript: {joined}"
    );
    assert!(
        joined.contains("native done"),
        "completion text in transcript: {joined}"
    );
}

/// （1.3）**新建**飞书会话（普通 `oc_*` 键）不再进内核：bridge 不判定为
/// native，dispatch 走既有 ACP 路径（发射 `Out::SpawnAcp`），无内核会话。
#[tokio::test]
async fn new_feishu_session_no_longer_routes_to_kernel() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_new_plain", None);
    let (router, mut out_rx) = DispatchHandle::new(map);

    let bridge = sebas::native_dispatch_bridge::DispatchNativeBridge::with_default(
        native_manager(),
        router.clone(),
        // 即便旧配置把 native_default 开成 true，新会话也不再进内核。
        true,
    );
    router.set_native_bridge(Some(bridge)).await;

    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "build it".into(),
            reply_target: None,
        })
        .await;

    // 走 ACP：out 流里出现 SpawnAcp（内核路径不发射它）。
    let mut saw_spawn = false;
    for _ in 0..16 {
        match tokio::time::timeout(Duration::from_millis(200), out_rx.recv()).await {
            Ok(Some(sebas_dispatch::Out::SpawnAcp { .. })) => {
                saw_spawn = true;
                break;
            }
            Ok(Some(_)) | Err(_) => continue,
            Ok(None) => break,
        }
    }
    assert!(saw_spawn, "新建飞书会话应走 ACP（SpawnAcp），不再进内核");
}
