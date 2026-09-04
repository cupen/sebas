//! 主运行编排：装配各子系统（router/manager/adapter 注册表）并跑到信号退出。
//!
//! 职责拆分：
//! - 出站分发 `Out` → 副作用： [`crate::dispatch`]
//! - ACP 会话生命周期（spawn/resume/pump）: [`crate::session_boot`]
//! - 通道适配器装配（decouple-feishu-channel task 4）：按配置实例化已启用
//!   adapter（飞书 = [`sebas_feishu::adapter::FeishuAdapter`]）填入
//!   [`sebas_channels::AdapterRegistry`]，入站 `ChannelEvent` 经 inbound
//!   通道交给 router；飞书 WS 循环由 adapter 自己拥有。
//!
//! 下面的 re-export 是 facade：integration tests 与 `replay` 仍走
//! `sebas::run::{...}` 路径，拆模块不牵动调用方。

pub use crate::session_boot::{
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts, restore_session_map,
    spawn_acp_pump,
};
pub use crate::ws_loop::{RouterEventHandler, ingest_feishu_frame};

use crate::config::{AgentConfig, Config};
use crate::dispatch::{dispatch_out, dispatch_out_without_feishu};
use crate::error::Result;
use crate::reactions::ReactionTracker;
use crate::ws_loop::spawn_test_session;
use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_acp::{AcpDriver, AgentDriver, ClaudeDriver};
use sebas_channels::key::ChannelName;
use sebas_channels::AdapterRegistry;
use sebas_feishu::adapter::{FeishuAdapter, FeishuAdapterConfig};
use sebas_feishu::client::FeishuClient;
use sebas_feishu::messages::{ReceiveIdType, SendTextRequest};
use sebas_gateway::config::GatewayConfig;
use sebas_router::router::RouterHandle;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Assemble the kind → driver registry the `SessionManager` routes sessions
/// through. Driver selection is closed (Claude → dedicated, Acp → generic);
/// adding a new native-ACP agent only changes config, not this function.
fn build_agent_registry(cfg: &Config) -> HashMap<String, AgentEntry> {
    cfg.acp
        .agents
        .iter()
        .map(|(slug, agent_cfg)| {
            let driver: Arc<dyn AgentDriver> = match agent_cfg {
                AgentConfig::Claude(_) => Arc::new(ClaudeDriver),
                AgentConfig::Acp { .. } => Arc::new(AcpDriver),
            };
            let entry = AgentEntry {
                driver,
                startup_timeout: cfg.acp.startup_timeout_for(slug),
            };
            (slug.clone(), entry)
        })
        .collect()
}

pub async fn run(
    cfg: Config,
    raw_config: String,
    test_msg: Option<String>,
    dump_inbound: Option<String>,
    gateway_cfg: Option<GatewayConfig>,
    webui: bool,
    webui_port: u16,
) -> Result<()> {
    // 在 watchdog 下运行时初始化 IPC
    if crate::ipc::is_under_watchdog() {
        init_watchdog_ipc().await;
    }

    // openlark 0.19 uses reqwest 0.13, whose Rustls connector consults the
    // process-wide provider. Our reqwest 0.12 clients use ring explicitly;
    // install one provider up front so the mixed dependency graph is
    // deterministic instead of panicking when both providers are compiled.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    init_tracing(&cfg);

    // openspec/specs/cli-service/spec.md startup checks: directories writable + ACP binary reachable.
    // Friendly Config error, no panic; runs before any network/spawn work.
    cfg.validate_runtime()?;

    // `run --gateway`：在随机端口上启动内置 gateway，实际端口记入日志
    // （调用方按需把 ANTHROPIC_BASE_URL/OPENAI_BASE_URL 指向该地址）。
    if let Some(ref gw_cfg) = gateway_cfg {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| crate::error::SebasError::Gateway(format!("绑定随机端口失败: {e}")))?;
        let (addr, _handle) = sebas_gateway::server::serve_with_listener(gw_cfg.clone(), listener)
            .map_err(|e| crate::error::SebasError::Gateway(e.to_string()))?;
        info!(%addr, "gateway started (run --gateway); point ANTHROPIC_BASE_URL/OPENAI_BASE_URL at {}", format!("http://{addr}"));
    }

    // feishu 是可选项（sebas-2ty / make-feishu-optional-webui-primary）：
    // 显式 `[feishu] enabled` 优先，缺省回退 app_id/app_secret 双非空判定。
    let feishu_enabled = cfg.feishu.is_enabled();
    if feishu_enabled && cfg.feishu.owner_id.is_empty() {
        // owner_id 决策（sebas-nya，文档化于 config.rs validate）：可选。
        // 空值 = 不过滤发送者 —— 对能执行任意命令的单用户 bot 是真实风险，
        // 启动时必须醒目提示。
        warn!(
            "feishu.owner_id 为空：任何飞书用户的消息都会被处理并驱动本机 claude；\
             单用户机器人建议配置 owner_id（openspec/specs/cli-service/spec.md）"
        );
    }
    if !feishu_enabled {
        // 显式 enabled = false 但凭据齐全：提示「刻意停用」而非「没配置」，
        // 让部署者明白为什么没有接入飞书（make-feishu-optional-webui-primary）。
        if cfg.feishu.enabled == Some(false) && (!cfg.feishu.app_id.is_empty() || !cfg.feishu.app_secret.is_empty()) {
            info!(
                "feishu 显式停用（[feishu] enabled = false），凭据已配置但不接入；\
                 以本地/WebUI 主控形态运行"
            );
        } else {
            info!(
                "feishu 未启用（app_id/app_secret 为空）：跳过飞书接入，\
                 以本地服务形态运行；如需接入请在配置中填写凭证"
            );
        }
    }

    if !crate::ipc::is_under_watchdog()
        && std::env::var_os("SEBAS_CONTROL_SECRET")
            .map(|v| v.is_empty())
            .unwrap_or(true)
    {
        // 裸 core 启动：/upgrade / /rollback / /gateway 等需要 watchdog RPC 的命令
        // 在此模式下不可用。启动时给可执行提示，避免用户调用 /upgrade 后才发现。
        warn!(
            "当前为裸 core 启动模式（SEBAS_IPC 未设置 + SEBAS_CONTROL_SECRET 未配置）：\
             /upgrade、/rollback、/restart、/gateway 等命令需要 watchdog 转发，\
             在此模式下调用会失败。如需启用，请通过 `sebas` watchdog 启动 core（openspec/specs/watchdog/spec.md）"
        );
    }

    let map = restore_session_map(&cfg.router.state_file, cfg.router.max_concurrent_sessions);

    // 5.5: 初始化状态库 DB (add-state-store)。
    // 如果 DB 初始化失败, 退回到文件存储 (向后兼容)。
    {
        let raw = std::env::var("SEBAS_STATE_DB")
            .unwrap_or_else(|_| "~/.sebas/sebas.db".into());
        let expanded = sebas_router::state_store::expand_tilde(&raw);
        let path = std::path::PathBuf::from(&expanded);
        match crate::sebas_state::writer::StateWriter::start(path.clone()) {
            Ok(writer) => {
                let engine = Box::new(crate::sebas_state::engine::DbStateEngine::new(
                    writer.handle().clone(),
                ));
                sebas_router::state_store::init_engine(engine);
                tracing::info!(path = %path.display(), "state store DB 已初始化");
            }
            Err(e) => {
                tracing::warn!(error = %e, "state store DB 初始化失败, 使用文件存储");
            }
        }
    }

    // TOML is bootstrap; settings.json (if present) wins wholesale.
    // Strict: malformed settings.json refuses to start with a clear error.
    // Missing settings.json → fall back to TOML [card] so first-boot users
    // get the configured values rather than serde defaults.
    //
    // decouple-feishu-channel task 3/4：`settings.json` 由 router 的中立
    // `CardConfig` 读写（两面 serde 形状逐一相同）；这里把它转成 router
    // 需要的类型（serde 往返，零字段映射代码）。
    //
    // 当 state store DB 可用时, 优先从 DB 读 settings; 再回退到文件。
    let merged_card_cfg = if let Some(engine) = sebas_router::state_store::engine() {
        match engine.load_settings().await {
            Ok(Some(value)) => {
                // DB 中有 settings, 用 Value 反序列化回 router CardConfig
                match serde_json::from_value::<sebas_router::CardConfig>(value) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::warn!(error = %e, "DB settings 反序列化失败, 回退到文件");
                        fallback_settings(&cfg)
                    }
                }
            }
            Ok(None) => {
                // DB 无 settings, 回退到文件
                fallback_settings(&cfg)
            }
            Err(e) => {
                tracing::warn!(error = %e, "DB 读取 settings 失败, 回退到文件");
                fallback_settings(&cfg)
            }
        }
    } else {
        fallback_settings(&cfg)
    };

    let mgr = Arc::new(SessionManager::new(
        cfg.acp.default_kind().to_string(),
        build_agent_registry(&cfg),
    ));
    let provider_forms = crate::provider::build_form(&raw_config);
    // WebUI 设置页的快照配置是 feishu 渲染配置（`[card]`），与 router 镜像
    // 同形；从已合并的 router 镜像转回 feishu 类型。
    let webui_card_cfg: sebas_feishu::cards::CardConfig =
        serde_json::from_value(serde_json::to_value(&merged_card_cfg).expect("card config serializes"))
            .expect("card config round-trips between mirror shapes");
    // 原生内核 manager（make-feishu-optional-webui-primary）：webui 的
    // NativeAgentBackend 与飞书原生桥共享同一个执行面（LLM 通道/工具注册表/
    // 审批 hub）。凭据缺失时 manager 仍可建，spawn 时按 cause 拒绝并诚实降级。
    let (native_mgr, _native_cause) =
        crate::agent_backend::NativeAgentBackend::build_native_manager(
            cfg.acp.startup_timeout_for(cfg.acp.default_kind()),
        );
    // 先建 router（native = None），再构造桥（桥需要 router 句柄），最后注入——
    // 解决桥↔router 循环依赖。
    let (router, mut out_rx) = RouterHandle::new_with_provider_form(
        map,
        merged_card_cfg,
        cfg.router.channel_buffer,
        provider_forms,
        Some(mgr.clone()),
    );
    let native_bridge = crate::native_router_bridge::RouterNativeBridge::with_default(
        native_mgr,
        router.clone(),
        cfg.feishu.native_default,
    );
    router.set_native_bridge(Some(native_bridge)).await;
    // Tracks the current emoji reaction on each session's root card so the
    // router's phase machine can swap 👀→🚧→✅ rather than pile them up.
    let reactions = Arc::new(ReactionTracker::default());

    // ── WS dump dir（adapter 入站快照用）──
    // feishu 未启用时不创建目录。
    let ws_dump_dir = match dump_inbound.as_ref() {
        Some(p) if feishu_enabled => match std::fs::create_dir_all(p) {
            Ok(()) => Some(std::path::PathBuf::from(p)),
            Err(e) => {
                warn!(?e, path = %p, "failed to create inbound dump dir; disabling dump");
                None
            }
        },
        _ => None,
    };
    if let Some(d) = &ws_dump_dir {
        info!(dir = %d.display(), "inbound WS payloads will be dumped here");
    }

    // feishu 启用：建 client、取 token、发 hello/test、spawn 出站 pump。
    // feishu 未启用（sebas-2ty）：全部跳过，出站接收端直接丢弃（与
    // standalone WebUI 同语义：Out 事件静默丢弃），进程等关闭信号。
    let http = reqwest::Client::new();
    // ── 适配器注册表装配（decouple-feishu-channel task 4/5）──
    // `web` 通道常驻注册（webui 的入站面是 HTTP API → SessionBackend，无
    // 传输循环）；飞书 adapter 按下方 `[feishu] enabled` 门禁注册。注册表
    // 回答"哪些通道活跃"（启动时打日志），渲染/传输细节归各 adapter。
    let mut registry = AdapterRegistry::new();
    registry.register(Box::new(sebas_webui::web_adapter::WebAdapter));

    if feishu_enabled {
        // Test affordance: `SEBAS_TEST_FAKE_TOKEN=1` skips the live Feishu auth
        // HTTP call and substitutes a stub token. Used by integration tests that
        // cannot reach the live Feishu API. Off by default; production callers
        // see no behaviour change.
        let tokens = if std::env::var("SEBAS_TEST_FAKE_TOKEN").as_deref() == Ok("1") {
            info!("SEBAS_TEST_FAKE_TOKEN=1; using stub tenant_access_token");
            sebas_feishu::client::TokenManager::with_stub_token("t-stub-test")
        } else {
            let tm = sebas_feishu::client::TokenManager::new(
                cfg.feishu.app_id.clone(),
                cfg.feishu.app_secret.clone(),
            );
            // Startup auth check stays fatal (openspec/specs/acp-driver/spec.md) — 仅在 feishu 启用时。
            tm.token()
                .await
                .map_err(|e| crate::error::SebasError::Feishu(e.to_string()))?;
            tm
        };

        // hello_msg: send to the owner (private DM via open_id) if both are set.
        // If owner_id is empty, do nothing.
        if !cfg.feishu.hello_msg.is_empty() && !cfg.feishu.owner_id.is_empty() {
            let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
            let req = SendTextRequest::new(
                &cfg.feishu.owner_id,
                ReceiveIdType::OpenId,
                &cfg.feishu.hello_msg,
            );
            let body = serde_json::to_value(&req).unwrap_or_default();
            let bearer = tokens.token().await.unwrap_or_default();
            match http.post(url).bearer_auth(&bearer).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    info!(%status, body = %body, "hello_msg send result");
                }
                Err(e) => warn!(?e, "hello_msg send failed"),
            }
        }

        // Optional startup test message: send "sebas 已启动" to the given receive_id
        // (interpreted as chat_id; for private DMs to a user, pass their open_id and
        // set receive_id_type=open_id below). Default to chat_id for groups.
        if let Some(receive_id) = test_msg {
            let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id";
            let req = SendTextRequest::new(receive_id, ReceiveIdType::ChatId, "✅ sebas 已启动");
            let body = serde_json::to_value(&req).unwrap_or_default();
            async {
                let bearer = tokens.token().await.unwrap_or_default();
                let resp = http
                    .post(url)
                    .bearer_auth(&bearer)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| crate::error::SebasError::Feishu(format!("send: {e}")))?;
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                info!(%status, body = %body, "test message send result");
                if !status.is_success() {
                    Err(crate::error::SebasError::Feishu(format!(
                        "test message failed: {body}"
                    )))
                } else {
                    Ok(())
                }
            }
            .await?;
        }

        // ── 适配器注册表装配（decouple-feishu-channel task 4）──
        // 按配置实例化飞书 adapter，注册进 `AdapterRegistry`；入站
        // `ChannelEvent` 经 `inbound` 通道交给 core（router）。WS 循环由
        // adapter 自己拥有（`spawn` 内部启动），core 不再硬编码启动 WS。
        let feishu = FeishuClient::new(sebas_feishu::client::FeishuConfig {
            app_id: cfg.feishu.app_id.clone(),
            app_secret: cfg.feishu.app_secret.clone(),
            owner_id: cfg.feishu.owner_id.clone(),
        });
        let adapter = FeishuAdapter::new(
            feishu.clone(),
            FeishuAdapterConfig {
                app_id: cfg.feishu.app_id.clone(),
                app_secret: cfg.feishu.app_secret.clone(),
                owner_id: cfg.feishu.owner_id.clone(),
                allowed_chat_types: cfg.feishu.allowed_chat_types.clone(),
                bot_name: cfg.feishu.bot_name.clone(),
                dump_dir: ws_dump_dir,
                // `[card]` 渲染配置：由 adapter 解释（theme/truncation/fold）。
                card_config: webui_card_cfg.clone(),
            },
        );
        registry.register(Box::new(adapter.clone()));
        let (inbound_tx, mut inbound_rx) = tokio::sync::mpsc::channel::<sebas_channels::ChannelEvent>(
            cfg.router.channel_buffer,
        );
        // 每个 adapter 一个 inbound 扇出：core 侧一个消费者任务，把
        // `ChannelEvent` 交给 router.dispatch。
        let inbound_router = router.clone();
        tokio::spawn(async move {
            while let Some(evt) = inbound_rx.recv().await {
                inbound_router.dispatch(evt).await;
            }
        });
        if let Some(feishu_adapter) = registry.get(&ChannelName::FEISHU.into()) {
            if let Err(e) = feishu_adapter.spawn(inbound_tx) {
                error!(error = %e, "failed to spawn feishu adapter; continuing without inbound");
            } else {
                info!("feishu adapter registered + WS loop spawned");
            }
        }
        // Spawn outbound pump
        let cfg_for_outbound = cfg.clone();
        let tokens_for_outbound = tokens;
        let http_for_outbound = http;
        let router_for_outbound = router.clone();
        let mgr_for_outbound = mgr.clone();
        let reactions_for_outbound = reactions;
        let gateway_cfg_for_outbound = gateway_cfg.clone();
        tokio::spawn(async move {
            while let Some(out) = out_rx.recv().await {
                if let Err(e) = dispatch_out(
                    &feishu,
                    &http_for_outbound,
                    &tokens_for_outbound,
                    &cfg_for_outbound,
                    &router_for_outbound,
                    &mgr_for_outbound,
                    &reactions_for_outbound,
                    gateway_cfg_for_outbound.as_ref(),
                    out,
                )
                .await
                {
                    error!(?e, "outbound dispatch failed");
                }
            }
        });
    } else {
        // feishu 未启用（sebas-2ty）：出站泵照样要跑 —— WebUI 的
        // spawn/消息/恢复（Out::WebSpawn / SendAcp / SpawnResume）由
        // no-feishu 分发器驱动真实 ACP 会话；卡片/reaction 等聊天向
        // Out 丢弃（没有聊天目的地）。此前直接 drop(out_rx)，导致
        // `run --webui` 的会话永远停在 Starting。
        let cfg_for_outbound = cfg.clone();
        let router_for_outbound = router.clone();
        let mgr_for_outbound = mgr.clone();
        let gateway_cfg_for_outbound = gateway_cfg.clone();
        tokio::spawn(async move {
            while let Some(out) = out_rx.recv().await {
                if let Err(e) = dispatch_out_without_feishu(
                    &cfg_for_outbound,
                    &router_for_outbound,
                    &mgr_for_outbound,
                    gateway_cfg_for_outbound.as_ref(),
                    out,
                )
                .await
                {
                    error!(?e, "outbound dispatch (feishu-less) failed");
                }
            }
        });
    }

    // 注册表含全部活跃通道（web 常驻 + 按配置启用的飞书）。绑定到函数
    // 作用域让 adapter 实例活满进程生命周期；日志即 spec 的"registry 可
    // 查询哪些通道活跃"。
    info!(
        channels = ?registry.names().map(|n| n.as_str().to_string()).collect::<Vec<_>>(),
        "active channel adapters"
    );
    let _registry = registry;

    // Start WebUI dashboard server if requested
    if webui {
        // The core IS this process: serve the dashboard over the in-process
        // session backend (no SessionManager — spawn/close dispatch through
        // the router's outbound pump).
        // 双执行后端：Claude Code 桥（acp）+ 原生内核（native），会话行创建
        // 时按 backend 提示选择（openspec/changes/sebas-agent-next 5.1/5.2）。
        let native = crate::agent_backend::NativeAgentBackend::from_env(
            cfg.acp.startup_timeout_for(cfg.acp.default_kind()),
        );
        let backend: std::sync::Arc<dyn sebas_webui::SessionBackend> =
            crate::agent_backend::DualSessionBackend::new(
                std::sync::Arc::new(sebas_webui::session_backend::InProcessBackend::new(
                    router.clone(),
                )),
                native,
            );
        let gateway_info = build_gateway_info(gateway_cfg.as_ref());
        // 创建会话下拉的可达 agent 列表：从 `cfg.acp.agents` 提取 (slug, argv)。
        let agent_kinds: Vec<sebas_webui::agent_kinds::AgentKindSource> = cfg
            .acp
            .agents
            .keys()
            .map(|slug| sebas_webui::agent_kinds::AgentKindSource {
                slug: slug.clone(),
                command: cfg.acp.command_for(slug).unwrap_or_default(),
            })
            .collect();
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{webui_port}"))
            .await
            .map_err(|e| crate::error::SebasError::Gateway(format!("绑定 webui 端口失败: {e}")))?;
        tokio::spawn(async move {
            sebas_webui::run(backend, gateway_info, webui_card_cfg, agent_kinds, listener).await;
        });
        info!("webui dashboard starting on 127.0.0.1:{webui_port}");
    }

    // Core session channel (5.9): the watchdog injects `SEBAS_CORE_SECRET`
    // into the core child (and the standalone WebUI), so its presence marks
    // "this process is the core under the watchdog" — the bare `sebas run`
    // keeps no socket, matching the client-side gate in webui_cmd.rs. `serve`
    // owns the socket lifecycle (bind, reclaim stale, remove on shutdown).
    let channel_shutdown = if !std::env::var("SEBAS_CORE_SECRET").unwrap_or_default().is_empty() {
        let core_secret = std::env::var("SEBAS_CORE_SECRET").unwrap_or_default();
        let channel_path = crate::core_channel::socket_path(&cfg);
        info!(path = %channel_path.display(), "core session channel listening");
        let channel_router = router.clone();
        let (close_tx, close_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            match crate::core_channel::server::serve(
                channel_router,
                channel_path,
                core_secret,
                close_rx,
            )
            .await
            {
                Ok(()) => info!("core session channel closed"),
                Err(e) => warn!(?e, "core session channel server exited"),
            }
        });
        Some(close_tx)
    } else {
        None
    };

    // Run the long-connection event loop inline in a `tokio::select!` so the
    // feishu 未启用时进程只等关闭信号（sebas-2ty）；WS 生命周期由 adapter
    // 的 spawn 任务拥有（见前面的注册表装配）。
    // Test affordance: `SEBAS_TEST_SPAWN_SESSION=1` mints a session via the
    // `acp.claude.path` binary at startup. Without this, the daemon idles
    // and no child is ever spawned — which makes the SIGTERM-cleanup test
    // vacuous. With it, an ACP child is alive as a direct descendant of
    // the sebas pid, so `kill_all` actually has work to do. Off by default.
    if std::env::var("SEBAS_TEST_SPAWN_SESSION").as_deref() == Ok("1") {
        spawn_test_session(&cfg, &router, &mgr).await;
    }

    info!("sebas started; waiting for SIGINT/SIGTERM");
    let sigint = async {
        tokio::signal::ctrl_c().await.ok();
    };
    let sigterm = async {
        #[cfg(unix)]
        {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
            sig.recv().await;
        }
        #[cfg(not(unix))]
        {
            // Non-unix platforms only have ctrl_c equivalent; never fires
            // separately here. Block forever so the select arm stays inert.
            std::future::pending::<()>().await;
        }
    };
    // feishu WS 生命周期由 adapter 的 spawn 任务拥有（task 4）：进程只在
    // 信号上等待；WS 循环退出（重连/致命错误）不结束 core。
    tokio::select! {
        _ = sigint => {
            info!("shutting down (SIGINT)");
        }
        _ = sigterm => {
            info!("shutting down (SIGTERM)");
        }
    }

    // Ask the core session channel to close (the serve task then removes the
    // socket file itself); give it a moment so the file is gone before the
    // watchdog's restart probes the path.
    if let Some(tx) = channel_shutdown {
        let _ = tx.send(true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Snapshot state BEFORE killing children (openspec/specs/acp-driver/spec.md order: dump, then
    // shutdown_children). Dumping after kill_all would race the pumps'
    // teardown (terminal events strip mappings) and would lose the whole
    // snapshot if a child hangs the kill — the restored mappings are what
    // lazy respawn (openspec/specs/session-lifecycle/spec.md) works from.
    let json = router
        .dump_json()
        .await
        .map_err(|e| crate::error::SebasError::Router(e.to_string()))?;
    if let Err(e) = std::fs::write(&cfg.router.state_file, json) {
        warn!(?e, "failed to persist session state");
    }

    // Signal all live sessions to cancel and reap their child processes.
    mgr.kill_all().await;
    Ok(())
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(&cfg.log.level).unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = fmt().with_env_filter(filter);
    if let Some(ref path) = cfg.log.file
        && let Ok(file) = std::fs::File::create(path)
    {
        subscriber.with_writer(file).init();
        return;
    }
    // watchdog 下 stdout 是 IPC 管道（Ready-only 协议）：日志必须写 stderr
    // （父进程 inherit），否则读端在 ready 后排空时会与日志争用同一管道。
    if crate::ipc::is_under_watchdog() {
        subscriber.with_writer(std::io::stderr).init();
        return;
    }
    subscriber.init();
}

/// Build a GatewayInfo from the optional gateway config for the WebUI.
fn build_gateway_info(gateway_cfg: Option<&GatewayConfig>) -> sebas_webui::models::GatewayInfo {
    let Some(gw) = gateway_cfg else {
        return sebas_webui::models::GatewayInfo::default();
    };
    let providers = gw
        .providers
        .iter()
        .map(|(name, p)| sebas_webui::models::ProviderInfo {
            name: name.clone(),
            base_url_anthropic: p.base_url_anthropic.clone(),
            base_url_openai: p.base_url_openai.clone(),
        })
        .collect();
    sebas_webui::models::GatewayInfo {
        listen: Some(gw.listen.clone()),
        provider_count: gw.providers.len(),
        debug: gw.debug,
        has_auth: !gw.auth_token.is_empty(),
        providers,
    }
}

/// 回退到 settings.json 读取, 再回退到 TOML `[card]`。
fn fallback_settings(cfg: &Config) -> sebas_router::CardConfig {
    match sebas_router::settings::load_settings(&sebas_router::settings::settings_path()) {
        Ok(Some(s)) => s,
        Ok(None) => {
            serde_json::from_value(serde_json::to_value(&cfg.card).expect("card config serializes"))
                .expect("card config round-trips between mirror shapes")
        }
        Err(e) => {
            tracing::error!(error = %e, "settings.json 解析失败, 使用 TOML 兜底");
            serde_json::from_value(serde_json::to_value(&cfg.card).expect("card config serializes"))
                .expect("card config round-trips between mirror shapes")
        }
    }
}

/// 在 watchdog 下运行时向父进程发送 ready 握手（Ready-only 协议）。
/// 控制命令一律走 control RPC（Unix socket），pipe 不再承载命令。
async fn init_watchdog_ipc() {
    use tracing::info;

    let mut ipc = crate::ipc::ChildIpc::new();
    if let Err(e) = ipc.ready().await {
        tracing::warn!("watchdog IPC ready 发送失败: {e}");
        return;
    }
    info!("watchdog IPC 连接就绪");
}
