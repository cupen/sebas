//! feishu → 原生 sebas-agent 内核 → webui 呈现的端到端链路
//! （make-feishu-optional-webui-primary，design D2/D3）。
//!
//! 验证：feishu `FeishuIn::Text` 经 router dispatch 路由到
//! `DispatchNativeBridge`（default_native = true）→ 原生内核创建会话并登记
//! mapping → webui `InProcessBackend` snapshot 看到该 oc_* 会话 →
//! transcript（工具轨迹 + 收尾文本）可读 → 权限请求可经 webui 审查卡回填。

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::DispatchHandle;
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

#[tokio::test]
async fn feishu_native_session_appears_in_webui_snapshot() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_native_webui", None);
    let (router, mut out_rx) = DispatchHandle::new(map);
    // 丢弃 Out（原生路径不发 SpawnAcp/卡片）。
    tokio::spawn(async move { while out_rx.recv().await.is_some() {} });

    // 装配真实桥（core 侧 = run.rs 的装配方式）。
    let bridge = sebas::native_dispatch_bridge::DispatchNativeBridge::with_default(
        native_manager(),
        router.clone(),
        true, // feishu native_default = true
    );
    router.set_native_bridge(Some(bridge)).await;

    let backend = InProcessBackend::new(router.clone());

    // 审查卡流：bash 属于默认政策 Ask（fail-closed），需要 webui 回填
    // allow-once 才能继续 turn——这同时验证权限呈现链路。必须先订阅再
    // dispatch（broadcast 只转发订阅后的事件）。
    let mut notices = backend.permission_requests().expect("backend has notices");

    // feishu 文本 → dispatch → 走桥创建原生会话。
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

    // webui snapshot 应看到该 oc_* 会话（status active）。
    let row = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snap = backend.snapshot().await;
            if let Some(info) = snap.into_iter().find(|i| i.channel == key.channel_str() && i.key == key.reference) {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("native feishu session should appear in webui snapshot");
    assert_eq!(row.channel, "feishu");
    assert_eq!(row.key, "oc_native_webui");
    assert_eq!(row.status, "active");

    // transcript 可读：工具轨迹 + 收尾文本最终出现在 turn 流。轮询至 deadline，
    // 超时也把当前的 transcript 打出来便于诊断。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    // 循环体每轮先赋值再读，无需初值。
    let mut last: Vec<sebas_dispatch::TurnEntry>;
    loop {
        // webui InProcessBackend 的 trait 键是中立 ChannelKey；feishu 通道的
        // key 用引用（chat_id，无 thread）。
        let trait_key = sebas_channels::ChannelKey::feishu("oc_native_webui", None);
        last = backend.turns(trait_key, 0).await.unwrap_or_default();
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
    assert!(joined.contains("bash"), "tool trace in transcript: {joined}");
    assert!(
        joined.contains("native done"),
        "completion text in transcript: {joined}"
    );
}