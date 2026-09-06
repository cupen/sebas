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
pub use crate::ws_loop::{DispatchEventHandler, ingest_feishu_frame};

use crate::config::{AgentConfig, Config};
use crate::dispatch::dispatch_out_without_feishu;
use crate::error::Result;
use crate::ws_loop::spawn_test_session;
use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_acp::{AcpDriver, AgentDriver, ClaudeDriver};
use sebas_channels::AdapterRegistry;
use sebas_router::config::RouterConfig;
use sebas_dispatch::engine::DispatchHandle;
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
    mut router_cfg: Option<RouterConfig>,
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

    // `run --router`：在随机端口上启动内置 router，实际端口记入日志
    // （调用方按需把 ANTHROPIC_BASE_URL/OPENAI_BASE_URL 指向该地址）。
    // 实际地址回写 `router_cfg.listen`：WebUI 的 router BFF 用同一快照
    // 定位 admin 面（provider 管理页），拿配置默认值会打不到真实端口。
    if let Some(gw_cfg) = router_cfg.as_mut() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| crate::error::SebasError::Router(format!("绑定随机端口失败: {e}")))?;
        let (addr, _handle) = sebas_router::server::serve_with_listener(gw_cfg.clone(), listener)
            .map_err(|e| crate::error::SebasError::Router(e.to_string()))?;
        gw_cfg.listen = addr.to_string();
        info!(%addr, "router started (core --router); point ANTHROPIC_BASE_URL/OPENAI_BASE_URL at {}", format!("http://{addr}"));
    }

    // extract-im-service M3：core 不再接入任何 IM。飞书装配（token/WS/
    // 卡片呈现）整体移至 `sebas im` 独立服务；`[feishu]` 配置节由 im 消费，
    // core 解析后不据此建立任何连接（im-service spec「配置归属」）。

    if !crate::ipc::is_under_watchdog()
        && std::env::var_os("SEBAS_CONTROL_SECRET")
            .map(|v| v.is_empty())
            .unwrap_or(true)
    {
        // 裸 core 启动：/upgrade / /rollback / /router 等需要 watchdog RPC 的命令
        // 在此模式下不可用。启动时给可执行提示，避免用户调用 /upgrade 后才发现。
        warn!(
            "当前为裸 core 启动模式（SEBAS_IPC 未设置 + SEBAS_CONTROL_SECRET 未配置）：\
             /upgrade、/rollback、/restart、/router 等命令需要 watchdog 转发，\
             在此模式下调用会失败。如需启用，请通过 `sebas` watchdog started core（openspec/specs/watchdog/spec.md）"
        );
    }

    let map = restore_session_map(&cfg.dispatch.state_file, cfg.dispatch.max_concurrent_sessions);

    // 5.5: 初始化状态库 DB (add-state-store)。
    // 如果 DB 初始化失败, 退回到文件存储 (向后兼容)。
    {
        let raw = std::env::var("SEBAS_STATE_DB")
            .unwrap_or_else(|_| "~/.sebas/sebas.db".into());
        let expanded = sebas_dispatch::state_store::expand_tilde(&raw);
        let path = std::path::PathBuf::from(&expanded);
        match crate::sebas_state::writer::StateWriter::start(path.clone()) {
            Ok(writer) => {
                let engine = Box::new(crate::sebas_state::engine::DbStateEngine::new(
                    writer.handle().clone(),
                ));
                sebas_dispatch::state_store::init_engine(engine);
                tracing::info!(path = %path.display(), "state store DB initialized");
            }
            Err(e) => {
                tracing::warn!(error = %e, "state store DB init failed, falling back to file storage");
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
    let merged_card_cfg = if let Some(engine) = sebas_dispatch::state_store::engine() {
        match engine.load_settings().await {
            Ok(Some(value)) => {
                // DB 中有 settings, 用 Value 反序列化回 router CardConfig
                match serde_json::from_value::<sebas_dispatch::CardConfig>(value) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to deserialize settings from DB, falling back to file");
                        fallback_settings(&cfg)
                    }
                }
            }
            Ok(None) => {
                // DB 无 settings, 回退到文件
                fallback_settings(&cfg)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to read settings from DB, falling back to file");
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
    // 原生内核 manager（make-feishu-optional-webui-primary）：webui、核心通道
    // server 与飞书原生桥共享同一个执行面（LLM 通道/工具注册表/审批 hub）。
    // 凭据缺失时 manager 仍可建，spawn 时按 cause 拒绝并诚实降级（cause 由下方
    // 装配 webui_backend 时透传到 NativeAgentBackend）。`available_models` 与
    // `default_model`（wire-webui-sebas-agent-e2e D5）同样在装配期确定，全
    // env（`SEBAS_AGENT_MODELS` / `SEBAS_AGENT_MODEL`）与内核 SessionConfig
    // 共用同一来源。
    let (native_mgr, native_cause, native_available_models, native_default_model) =
        crate::agent_backend::NativeAgentBackend::build_native_manager(
            cfg.acp.startup_timeout_for(cfg.acp.default_kind()),
        );
    // 先建 router（native = None），再构造桥（桥需要 router 句柄），最后注入——
    // 解决桥↔router 循环依赖。
    let (router, mut out_rx) = DispatchHandle::new_with_provider_form(
        map,
        merged_card_cfg,
        cfg.dispatch.channel_buffer,
        provider_forms,
        Some(mgr.clone()),
    );
    let native_bridge = crate::native_dispatch_bridge::DispatchNativeBridge::with_default(
        native_mgr.clone(),
        router.clone(),
        cfg.feishu.native_default,
    );
    router.set_native_bridge(Some(native_bridge)).await;
    // ── 适配器注册表：`web` 常驻（webui 的入站面是 HTTP API →
    // SessionBackend，无传输循环）。core 不注册任何 IM 适配器。
    let mut registry = AdapterRegistry::new();
    registry.register(Box::new(sebas_webui::web_adapter::WebAdapter));

    // ── 出站泵（无 IM 呈现）──
    // extract-im-service M3：core 的出站只驱动会话执行（WebSpawn / SendAcp /
    // SpawnResume）；卡片/reaction 等聊天向 Out 由 `sebas im` 的前端经通道
    // 数据自行渲染（design D3），在 core 侧无目的地、静默丢弃。
    let cfg_for_outbound = cfg.clone();
    let router_for_outbound = router.clone();
    let mgr_for_outbound = mgr.clone();
    let router_cfg_for_outbound = router_cfg.clone();
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            if let Err(e) = dispatch_out_without_feishu(
                &cfg_for_outbound,
                &router_for_outbound,
                &mgr_for_outbound,
                router_cfg_for_outbound.as_ref(),
                out,
            )
            .await
            {
                error!(?e, "outbound dispatch failed");
            }
        }
    });

    // 注册表：web 常驻（webui 入站面）。extract-im-service M3 后 core 不再
    // 注册任何 IM 适配器（im-service spec「多 IM 适配器宿主」）。
    info!(
        channels = ?registry.names().map(|n| n.as_str().to_string()).collect::<Vec<_>>(),
        "active channel adapters"
    );
    let _registry = registry;

    // Start WebUI dashboard server if requested. 双执行后端（Claude Code 桥 +
    // 原生内核；sebas-agent-next 5.1/5.2）。核心常驻一份，webui 与核心通道
    // server 共享（design D1 of wire-webui-sebas-agent-e2e）：复用上方为飞书
    // 原生桥构建的同一个 `native_mgr`，detached 形态下经通道 spawn 的 native 会
    // 话与 in-process 看到的是同一个内核 manager。
    let webui_backend: std::sync::Arc<dyn sebas_webui::SessionBackend> =
        crate::agent_backend::DualSessionBackend::new(
            std::sync::Arc::new(sebas_webui::session_backend::InProcessBackend::new(
                router.clone(),
            )),
            crate::agent_backend::NativeAgentBackend::with_manager_arc(
                native_mgr.clone(),
                native_cause.clone(),
                native_available_models,
                native_default_model,
            ),
        );
    if webui {
        // The core IS this process: serve the dashboard over the in-process
        // session backend (no SessionManager — spawn/close dispatch through
        // the router's outbound pump).
        let backend = webui_backend.clone();
        let router_info = build_router_info(router_cfg.as_ref());
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
        // add-webui-picker-workdir-start：browse-dirs 的服务端默认浏览根 =
        // 默认 agent kind 的 work_dir（dispatch 的会话 work dir 回退语义），
        // 未配置时回退进程 cwd；两者皆无 → browse-dirs 硬错误。
        let webui_work_root = match cfg.acp.work_dir_for(cfg.acp.default_kind()) {
            Some(dir) => Some(std::path::PathBuf::from(dir)),
            None => std::env::current_dir().ok(),
        };
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{webui_port}"))
            .await
            .map_err(|e| crate::error::SebasError::Router(format!("绑定 webui 端口失败: {e}")))?;
        let webui_auth = cfg.watchdog.webui.auth;
        tokio::spawn(async move {
            // 登录鉴权与独立 webui 进程同一套（add-webui-auth-switch）：
            // 开关关闭 → disabled 态全路由免登录；打开 → 凭据文件 + env 引导。
            // core --webui 恒绑 127.0.0.1，不受非 loopback 门影响。
            let auth = if webui_auth {
                crate::webui_cmd::bootstrap_auth()
            } else {
                tracing::warn!("webui auth disabled via [watchdog.webui] auth = false: all routes are public");
                std::sync::Arc::new(sebas_webui::auth::AuthHandle::disabled())
            };
            sebas_webui::run_with_admin_adapter_and_auth(
                backend,
                router_info,
                webui_card_cfg,
                agent_kinds,
                listener,
                None,
                auth,
                webui_work_root,
            )
            .await;
        });
        info!("webui dashboard starting on 127.0.0.1:{webui_port}");
    }

    // Core session channel (5.9): the watchdog injects `SEBAS_CORE_SECRET`
    // into the core child (and the standalone WebUI), so its presence marks
    // "this process is the core under the watchdog" — the bare `sebas core`
    // keeps no socket, matching the client-side gate in webui_cmd.rs. `serve`
    // owns the socket lifecycle (bind, reclaim stale, remove on shutdown).
    // Session mutations are delegated to the composite backend (design D1),
    // so detached WebUI and `core --webui` see identical behavior.
    let channel_shutdown = if !std::env::var("SEBAS_CORE_SECRET").unwrap_or_default().is_empty() {
        let core_secret = std::env::var("SEBAS_CORE_SECRET").unwrap_or_default();
        let channel_path = crate::core_channel::socket_path(&cfg);
        info!(path = %channel_path.display(), "core session channel listening");
        let channel_router = router.clone();
        let channel_backend = webui_backend.clone();
        let (close_tx, close_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            match crate::core_channel::server::serve(
                channel_backend,
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
        .map_err(|e| crate::error::SebasError::Dispatch(e.to_string()))?;
    if let Err(e) = std::fs::write(&cfg.dispatch.state_file, json) {
        warn!(?e, "failed to persist session state");
    }

    // Signal all live sessions to cancel and reap their child processes.
    mgr.kill_all().await;
    Ok(())
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(format!("{}{}", cfg.log.level, crate::config::LOG_FILTER_QUIET))
        .unwrap_or_else(|_| EnvFilter::new(crate::config::DEFAULT_LOG_FILTER));
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

/// Build a RouterInfo from the optional router config for the WebUI.
/// fix-webui-detached-status：pub(crate) 供 standalone webui（`sebas webui`）
/// 复用同一装配——detached 形态不再以 `RouterInfo::default()` 占位。
pub(crate) fn build_router_info(
    router_cfg: Option<&RouterConfig>,
) -> sebas_webui::models::RouterInfo {
    let Some(gw) = router_cfg else {
        return sebas_webui::models::RouterInfo::default();
    };
    let providers = gw
        .providers
        .iter()
        .map(|(name, p)| sebas_webui::models::ProviderInfo {
            name: name.clone(),
            preset: p.preset.clone(),
            base_url_anthropic: p.base_url_anthropic.clone(),
            base_url_openai_chat: p.base_url_openai_chat.clone(),
            base_url_openai_responses: p.base_url_openai_responses.clone(),
        })
        .collect();
    sebas_webui::models::RouterInfo {
        listen: Some(gw.listen.clone()),
        provider_count: gw.providers.len(),
        debug: gw.debug,
        has_auth: !gw.auth_token.is_empty(),
        providers,
    }
}

/// 回退到 settings.json 读取, 再回退到 TOML `[card]`。
fn fallback_settings(cfg: &Config) -> sebas_dispatch::CardConfig {
    match sebas_dispatch::settings::load_settings(&sebas_dispatch::settings::settings_path()) {
        Ok(Some(s)) => s,
        Ok(None) => {
            serde_json::from_value(serde_json::to_value(&cfg.card).expect("card config serializes"))
                .expect("card config round-trips between mirror shapes")
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to parse settings.json, falling back to TOML");
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
        tracing::warn!("failed to send watchdog IPC ready: {e}");
        return;
    }
    info!("watchdog IPC connected");
}

#[cfg(test)]
mod router_info_tests {
    use super::build_router_info;
    use sebas_router::config::{RouterConfig, ProviderConfig};
    use std::collections::HashMap;

    // fix-webui-detached-status 2.1：detached webui 与 in-process 共用同一
    // 装配。直接构造 RouterConfig，不走 env 敏感的 parse（并行测试会改
    // SEBAS_ROUTER_PROVIDER_OVERLAY，污染 parse）。
    fn gw_config() -> RouterConfig {
        RouterConfig {
            listen: "127.0.0.1:50770".into(),
            max_body_bytes: 1024,
            connect_timeout_secs: 5,
            read_timeout_secs: 5,
            usage_file: String::new(),
            debug: false,
            provider_overlay: String::new(),
            default_provider: None,
            auth_token: vec!["tok".into()],
            rate_limit: Default::default(),
            providers: HashMap::from([(
                "anthropic".into(),
                ProviderConfig {
                    preset: Some("anthropic".into()),
                    base_url_anthropic: Some("https://api.anthropic.com".into()),
                    base_url_openai_chat: None,
                    base_url_openai_responses: None,
                    api_key_env: None,
                    api_key: Some("sk-x".into()),
                    model_map: HashMap::new(),
                    models: vec![],
                },
            )]),
            routes: vec![],
            model_aliases: HashMap::new(),
            config_source: String::new(),
        }
    }

    #[test]
    fn router_config_populates_info() {
        let info = build_router_info(Some(&gw_config()));
        assert_eq!(info.listen.as_deref(), Some("127.0.0.1:50770"));
        assert_eq!(info.provider_count, 1);
        assert_eq!(info.providers[0].name, "anthropic");
        assert!(info.has_auth);
        assert!(!info.debug);
    }

    // 无 router 配置（纯会话核心）→ default：这是"真的没配 router"，
    // 与 detached 占位缺陷不同。
    #[test]
    fn missing_router_section_falls_back_to_default() {
        let info = build_router_info(None);
        assert_eq!(info.listen, None);
        assert_eq!(info.provider_count, 0);
        assert!(info.providers.is_empty());
    }
}
