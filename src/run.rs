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
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts, spawn_acp_pump,
};
pub use crate::ws_loop::{DispatchEventHandler, ingest_feishu_frame};

use crate::config::{AgentConfig, Config};
use crate::dispatch::dispatch_out_without_feishu;
use crate::error::Result;
use crate::ws_loop::spawn_test_session;
use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_acp::{AcpDriver, AgentDriver, ClaudeDriver};
use sebas_channels::AdapterRegistry;
use sebas_dispatch::engine::DispatchHandle;
use sebas_router::config::RouterConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
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
                // （workbench-composer-input-polish 2.1）claude 驱动实例携带
                // 各自的模型别名表：`[acp.agents.<slug>] models` 覆盖值，缺省
                // /空表回退内置（resolved_models 单点归一）。
                AgentConfig::Claude(c) => Arc::new(ClaudeDriver::with_models(c.resolved_models())),
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
    router_cfg: Option<RouterConfig>,
    webui: bool,
    webui_port: u16,
    webui_host: String,
    config_path: String,
) -> Result<()> {
    // 在 watchdog 下运行时初始化 IPC。3.1（D4）：IPC 句柄在此创建，但 ready
    // 信号延迟到核心通道 bind+武装完成之后——「Running」从此蕴含「已武装」。
    let mut watchdog_ipc = if crate::ipc::is_under_watchdog() {
        Some(crate::ipc::ChildIpc::new())
    } else {
        None
    };

    // openlark 0.19 uses reqwest 0.13, whose Rustls connector consults the
    // process-wide provider. Our reqwest 0.12 clients use ring explicitly;
    // install one provider up front so the mixed dependency graph is
    // deterministic instead of panicking when both providers are compiled.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    init_tracing(&cfg);

    // openspec/specs/cli-service/spec.md startup checks: directories writable + ACP binary reachable.
    // Friendly Config error, no panic; runs before any network/spawn work.
    cfg.validate_runtime()?;

    // unify-router-process-shape 1.1（D1）：内嵌 router 启动块已删除——core
    // 进程内不再有任何 router HTTP 面。`router_cfg` 只是配置声明的 `[router]`
    // 段（listen / auth_token / providers 种子），供 spawn env 翻译（
    // ProviderMode::Router 指向独立 router 进程）、出站分发与节点链路
    // RouterEndpoint 使用。随机端口回写与 `router started (core --router)`
    // 日志随形态消亡。

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

    // 5.5: 初始化状态库 (add-state-store)；single-state-dir D2/D5：core 的
    // 库按增长特征拆成 settings.db（providers/model_aliases/settings，有界）
    // 与 projects.db（projects/session_map，增长），两库各开一次、路径由
    // 状态目录映射表派生（SEBAS_STATE_DIR > SEBAS_HOME > ~/.sebas，逐库
    // 变量可显式覆盖）。
    // fail-fast-on-startup-errors 任务 1.3：settings 库不可写是启动失败
    // （它是配置主干，没有它 provider/settings 域无从谈起）；projects 库
    // 不可用则**该域降级并如实报告**（state-store spec「unavailability is
    // reported, not hidden」）——项目面每个操作返回点名原因的 typed
    // rejection，设置面照常工作，不拿空表冒充现状。
    // persist-session-map：会话映射的恢复源从 `[dispatch] state_file` 改为
    // 状态库——放在 init_engine 之后（恢复读的就是 projects.db），映射空表
    // /条目不可读绝不阻塞启动（session-lifecycle「Restart recovery with
    // corruption tolerance」；库本身打不开归状态库的损坏规则管辖）。
    {
        // 退休变量提示（single-state-dir 6.1）：SEBAS_STATE_DB 随单库退休，
        // 不再被读取；残留值只提示，不改变任何行为。
        let retired = sebas_domain::state_paths::retired_env_vars_present();
        for var in retired {
            tracing::warn!(
                "{var} 已退休：sebas.db 不复存在，该变量不会被读取。\
                 状态落点改由 SEBAS_STATE_DIR（状态目录）派生，逐库覆盖变量 \
                 （SEBAS_SETTINGS_DB / SEBAS_PROJECTS_DB / SEBAS_WEBUI_AUTH_DB / \
                 SEBAS_ROUTER_USAGE_DB）按需显式设置。"
            );
        }
        // 退休变量提示（retire-legacy-state-json 3.3）：遗留状态文件与它们的
        // 路径变量都已退休——文件不再被写、不再被读、也不再被导入，变量导出
        // 不改变任何行为。只报告，不读取其值。
        if !sebas_dispatch::state_store::retired_file_env_vars_present().is_empty() {
            tracing::warn!(
                "{} / {} 已退休：state.json 与 providers.json 不再被读写、也不\
                 会被导入，导出这两个变量不改变任何行为。provider 与运行态状态\
                 的唯一权威是状态库。",
                sebas_dispatch::state_store::RETIRED_STATE_FILE_VAR,
                sebas_dispatch::state_store::RETIRED_PROVIDER_OVERLAY_VAR,
            );
        }

        let settings_path = sebas_domain::state_paths::Database::Settings.resolve();
        let projects_path = sebas_domain::state_paths::Database::Projects.resolve();

        // 状态目录收紧为 owner-only（retire-legacy-state-json 1.1）：库文件本身
        // 在 `sebas_db::conn::open` 里收紧，目录没有单一 opener（archive.json /
        // services.json / nodes.json 各有其写入者），所以在状态目录的归属处显式
        // 收紧一次。失败只 warn，绝不阻断启动。
        if let Some(dir) = settings_path.parent() {
            sebas_db::conn::secure_directory(dir);
        }

        let settings_writer =
            match crate::sebas_state::writer::StateWriter::start_settings(settings_path.clone()) {
                Ok(writer) => writer,
                Err(e) => {
                    return Err(crate::error::SebasError::Config(format!(
                        "settings.db 不可写 ({}): {e}",
                        settings_path.display()
                    )));
                }
            };
        // legacy defaults.json 一次性导入 settings 域（make-core-own-provider-data
        // 1.4：标记在场即永不读该文件）。放在 init_engine 之前——经同一 writer
        // 句柄串行提交，不与后续引擎写并发。
        if let Err(e) = crate::sebas_state::defaults_import::import_legacy_defaults_once(
            settings_writer.handle(),
        )
        .await
        {
            tracing::warn!(error = %e, "legacy defaults 导入阶段失败（不阻断启动）");
        }

        let engine = match crate::sebas_state::writer::StateWriter::start_projects(
            projects_path.clone(),
        ) {
            Ok(projects_writer) => {
                Box::new(crate::sebas_state::engine::DbStateEngine::with_projects(
                    settings_writer.handle().clone(),
                    projects_writer.handle().clone(),
                )) as Box<dyn sebas_dispatch::state_store::StateStoreEngine + Send + Sync>
            }
            Err(e) => {
                tracing::error!(
                    path = %projects_path.display(),
                    error = %e,
                    "projects.db 打不开：项目域降级（操作将如实拒绝），设置域照常"
                );
                Box::new(crate::sebas_state::engine::DbStateEngine::with_unavailable_projects(
                    settings_writer.handle().clone(),
                    format!("{} ({})", projects_path.display(), e),
                )) as Box<dyn sebas_dispatch::state_store::StateStoreEngine + Send + Sync>
            }
        };
        sebas_dispatch::state_store::init_engine(engine);
        tracing::info!(
            settings = %settings_path.display(),
            projects = %projects_path.display(),
            "state store DBs initialized (settings.db + projects.db)"
        );
        // settings_writer 在此 drop：写者线程由引擎持有的 handle 克隆续命
        // （全部 sender 关闭才退出），与既有形态一致。
    }

    // 会话映射从状态库恢复（persist-session-map 2.1）：空表 → 空表启动；
    // 条目不可读（不可寻址行）→ 告警 + 跳过，恢复的条目一律 Dormant/占位，
    // 供惰性 respawn（openspec/specs/session-lifecycle/spec.md）。`capacity`
    // 接线 `[dispatch] max_concurrent_sessions`。
    let map = match sebas_dispatch::state_store::engine() {
        Some(engine) => match engine.load_session_map().await {
            Ok(rows) => sebas_dispatch::state::SessionMap::restore_rows(
                rows,
                cfg.dispatch.max_concurrent_sessions,
            ),
            Err(e) => {
                warn!(error = %e, "session map 读取失败：以空表启动（不阻塞启动）");
                sebas_dispatch::state::SessionMap::with_capacity(
                    cfg.dispatch.max_concurrent_sessions,
                )
            }
        },
        None => {
            warn!("state store 未初始化：会话映射以空表启动");
            sebas_dispatch::state::SessionMap::with_capacity(cfg.dispatch.max_concurrent_sessions)
        }
    };

    // close-acceptance-blind-spots 盲区 2：env posture 启动告警——继承自
    // shell 且不被 cover 语义覆盖的 ANTHROPIC_* 逐变量 WARN 点名（只报告，
    // 不篡改 env，design D1）。放在 state store 初始化之后：provider 解析
    // 与 spawn 路径同源（库权威）。
    crate::spawn_env::warn_inherited_provider_env(router_cfg.as_ref());

    // TOML `[card]` 是唯一引导值。
    //
    // retire-legacy-state-json 4.1：`settings.json` 已退休——card 配置的唯一
    // 持久权威是状态库 `settings` 表的 `card_config` 键；库不可用/无值时按
    // TOML `[card]` 呈现（绝不落到 serde 默认值）。
    //
    // decouple-feishu-channel task 3/4：这两个 `CardConfig` 是逐一字段相同的
    // serde 镜像形状；这里做一次 serde 往返，零字段映射代码。
    let merged_card_cfg = if let Some(engine) = sebas_dispatch::state_store::engine() {
        match engine.load_settings().await {
            Ok(Some(value)) => {
                // DB 中有 settings, 用 Value 反序列化回 router CardConfig
                match serde_json::from_value::<sebas_dispatch::CardConfig>(value) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::warn!(error = %e, "state store card_config 解析失败，回退 TOML [card]");
                        fallback_settings(&cfg)
                    }
                }
            }
            Ok(None) => {
                // 库里没有 card_config → TOML 引导值
                fallback_settings(&cfg)
            }
            Err(e) => {
                tracing::warn!(error = %e, "state store card_config 读取失败，回退 TOML [card]");
                fallback_settings(&cfg)
            }
        }
    } else {
        tracing::warn!(
            "{}；card 配置按 TOML [card] 引导值呈现",
            sebas_dispatch::state_store::unavailable_cause().unwrap_or("状态库不可用")
        );
        fallback_settings(&cfg)
    };

    let mgr = Arc::new(SessionManager::new(
        cfg.acp.default_kind().to_string(),
        build_agent_registry(&cfg),
    ));
    let provider_forms = crate::provider::build_form(&raw_config);
    // WebUI 设置页的快照配置是 feishu 渲染配置（`[card]`），与 router 镜像
    // 同形；从已合并的 router 镜像转回 feishu 类型。
    let webui_card_cfg: sebas_feishu::cards::CardConfig = serde_json::from_value(
        serde_json::to_value(&merged_card_cfg).expect("card config serializes"),
    )
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

    // ── 重启 spawning 收敛（close-acceptance-blind-spots 盲区 4，design D2
    // 重投优先）──恢复载入的 spawning 相位会话（0-turn 占位）在启动期落定：
    // 重投 spawn 指令，重投失败经既有 fail_spawn 落合成错误条目 + spawn-failed
    // 终态——恢复完成后不再有无人收敛的 spawning 僵尸。放在出站泵装配之后：
    // 重投指令经泵投递、每个 spawn 握手在独立任务里跑，单会话落定不阻塞其它
    // 会话恢复，也不阻塞启动主路径。
    {
        let router_for_settle = router.clone();
        tokio::spawn(async move {
            let settled = router_for_settle.settle_restored_spawning().await;
            if settled > 0 {
                info!("restore: 已重投 {settled} 个 spawning 相位会话的 spawn 指令");
            }
        });
    }

    // ── 回合停滞看门狗（fix-pending-queue-liveness 2.2，design D1/D6）──
    // `[dispatch] turn_stall_timeout`（默认 600s，0 = 关闭）配置进引擎；
    // 周期扫描复用出站泵的装配点。扫描间隔钳在 1–15s（≈阈值）：默认 600s
    // 用 15s 的巡检节奏，收尾迟滞 ≤ 一个间隔；小阈值供 e2e 快速周转。
    let stall_timeout = cfg.dispatch.turn_stall_timeout;
    router.set_turn_stall_timeout(stall_timeout);
    if stall_timeout > 0 {
        let stall_router = router.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(stall_timeout.clamp(1, 15)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                stall_router.force_settle_stalled_turns().await;
            }
        });
    }

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
        // 排序确定性（workbench-agent-wire-fix）：HashMap 迭代序随机，「首个
        // 可达 agent」预选不能在进程间漂移——default kind 最先，其余字典序。
        let mut agent_slugs: Vec<&String> = cfg.acp.agents.keys().collect();
        agent_slugs.sort();
        let default_kind = cfg.acp.default_kind().to_string();
        agent_slugs.sort_by_key(|s| s.as_str() != default_kind.as_str());
        let agent_kinds: Vec<sebas_webui::agent_kinds::AgentKindSource> = agent_slugs
            .into_iter()
            .map(|slug| {
                let driver = cfg.acp.driver_tag_of(slug);
                sebas_webui::agent_kinds::AgentKindSource {
                    slug: slug.clone(),
                    command: cfg.acp.command_for(slug).unwrap_or_default(),
                    driver,
                    display: cfg.acp.display_for(slug),
                }
            })
            .collect();
        // add-workspace-root：恒有值的单一机器级边界，env（SEBAS_WORKSPACE_ROOT）
        // > config（[workspace] root）> cwd 回退；回退生效时打一条启动告警
        // （D5：告警在装配处打——判定函数被高频调用，回退是启动期事实）。
        // browse-dirs 的起点与显式 root 约束、项目面范围判定都以它为准
        // （work_root / allowed_roots 双根形态已随白名单机制一并退役）。
        let (webui_workspace_root, workspace_root_fell_back) =
            crate::config::resolve_workspace_root(
                std::env::var("SEBAS_WORKSPACE_ROOT").ok().as_deref(),
                cfg.workspace.root.as_deref(),
                &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            );
        if workspace_root_fell_back {
            warn!(
                "workspace root 未显式配置，回退进程当前工作目录 {}；建议显式配置 workspace root（[workspace] root 或 SEBAS_WORKSPACE_ROOT）",
                webui_workspace_root.display()
            );
        }
        // add-system-dir-denylist 3.1：root 落在系统目录上只警示不阻断。
        crate::config::warn_if_workspace_root_is_system_dir(&webui_workspace_root);
        // 登录鉴权与独立 webui 进程同一套（add-webui-multiuser-rbac 3.4，
        // design D4）：开关关闭 → disabled 态全路由免登录；打开 → 建库 +
        // env 引导 root，零用户留给首启设置页。装配提到 bind 之前，让非
        // loopback 安全门先于 bind 完成裁决。
        let webui_auth = if cfg.service.webui.auth {
            crate::webui_cmd::bootstrap_auth()
        } else {
            tracing::warn!(
                "webui auth disabled via [service.webui] auth = false: all routes are public"
            );
            std::sync::Arc::new(sebas_webui::auth::AuthHandle::disabled())
        };
        // 非 loopback 安全门与独立 webui 进程同一裁决（webui_cmd::
        // ensure_non_loopback_bind_allowed）：`--webui-host` 可传 0.0.0.0 等
        // 公网地址，开关关闭 / 用户库无启用用户时在 bind 前硬失败——不给
        // 公网留裸奔端口或抢注 root 的窗口。默认 127.0.0.1 时门不触发。
        if !webui_host_is_loopback(&webui_host) {
            crate::webui_cmd::ensure_non_loopback_bind_allowed(
                cfg.service.webui.auth,
                &webui_auth,
            )?;
        }
        let listener = tokio::net::TcpListener::bind(format!("{webui_host}:{webui_port}"))
            .await
            .map_err(|e| crate::error::SebasError::Router(format!("绑定 webui 端口失败: {e}")))?;
        // add-agent-skills 5.1：skills 管理面的仓操作接缝——仓目录与
        // placement/no_placement 表从 config 装配（core 逻辑在
        // sebas::skills，CLI 与 webui 同源）。在 spawn **之前**装配：
        // async move 块里借用整个 cfg 会把它整个移进 future。
        let skills: std::sync::Arc<dyn sebas_webui::skills::SkillsService> =
            std::sync::Arc::new(crate::skills::FsSkillsService::from_config(&cfg));
        // preselect-last-used-model 3.2：default agent kind 与 agent 目录
        // 同一装配点注入（About INSTANCE 段经 /api/about 下发运行时真值）。
        let agent_kinds_provider: std::sync::Arc<dyn sebas_webui::agent_kinds::AgentKindProvider> =
            std::sync::Arc::new(
                sebas_webui::agent_kinds::ConfigAgentKindProvider::with_default_kind(
                    agent_kinds,
                    default_kind,
                ),
            );
        tokio::spawn(async move {
            sebas_webui::run_with_admin_adapter_and_auth(
                backend,
                router_info,
                webui_card_cfg,
                agent_kinds_provider,
                listener,
                None,
                webui_auth,
                webui_workspace_root,
                cfg.service.webui.archive_retention_days,
                skills,
            )
            .await;
        });
        info!("webui dashboard starting on http://{webui_host}:{webui_port}");
    }

    // Core session channel auto-arm（harden-core-channel-deployment 1.2/D3）：
    // 通道恒武装——`SEBAS_CORE_SECRET` 存在则沿用（watchdog 部署路径不变），
    // 缺失则现场生成随机 secret；无论哪种来源都原子写入 config 解析出的
    // secret 文件（0600），无 env 注入的客户端（standalone webui / im /
    // router 订阅）按 env → 文件顺序发现密钥。bind 在主路径完成且先于
    // watchdog ready（D4/3.1）：bind 失败在 ready 之前返回 Err → main 统一
    // 出口以 75 退出，socket 生命周期（accept 循环、优雅退出删 socket 文件）
    // 仍归 [`crate::core_channel::server::serve_bound`] 所有。
    // 节点注册表写者句柄（add-remote-execution-node 2.7）：必须在 arm_core_channel
    // **之前**打开——通道要拿同一个句柄承载节点管理入口（签发 token / 列节点 /
    // 吊销）。注册表是单写者文件，另开实例会与它互相覆盖。
    let node_registry = if cfg.node_link.enabled {
        Some(
            crate::node_link::runtime::open_registry(
                &cfg.node_link,
                std::path::Path::new(&config_path),
            )
            .map_err(|e| crate::error::SebasError::Config(format!("节点注册表不可用：{e}")))?,
        )
    } else {
        None
    };

    // 远端会话投影（add-remote-execution-node 5.1/5.3）：节点链路启用时才有——
    // 没有节点就没有远端会话，硬放一个空投影只会让快照多一次无谓的合并。
    let projection = node_registry
        .as_ref()
        .map(|_| crate::node_link::RemoteProjection::new());

    let armed_channel = arm_core_channel(
        &cfg,
        std::path::Path::new(&config_path),
        webui_backend.clone(),
        &router,
        node_registry.clone(),
        projection.clone(),
    )
    .await?;

    // 执行节点入站链路（add-remote-execution-node D13）：由 core 托管，默认关
    // （`[node_link] enabled = true` 才开）。bind 在 ready 之前完成，失败即启动
    // 失败（75），不顶着"监听没起来"继续对外服务。
    let armed_node_link = match &node_registry {
        None => None,
        Some(registry) => {
            // 接入/断开都通知投影：接入即按节点的事实对账（重建视图并续传），
            // 断开只把会话标成"暂时看不见"（**不终止**——执行事实在节点上）。
            let observer = projection
                .as_ref()
                .map(|p| crate::node_link::ProjectionObserver::new(std::sync::Arc::clone(p)));
            // 内置 router 的地址只有主控知道（随机端口）；把它与主控 router 的
            // 下游凭据一并告知节点——节点因此**零 provider 凭据**也能出网（7.2）。
            // 没启用内置 router 时是 None：节点配了 control-plane-router 会如实拒绝，
            // 而不是猜一个地址去撞。
            let router_endpoint = crate::node_link::server::RouterEndpoint {
                url: router_cfg.as_ref().map(|c| format!("http://{}", c.listen)),
                token: router_cfg
                    .as_ref()
                    .and_then(|c| c.auth_token.first().cloned()),
            };
            let served = crate::node_link::runtime::serve_registry(
                &cfg.node_link,
                std::sync::Arc::clone(registry),
                observer,
                router_endpoint,
            )
            .await
            .map_err(|e| crate::error::SebasError::Config(format!("节点链路监听失败：{e}")))?;
            // 首配 token 在**通道与监听都已 bind 之后**才签发：否则通道 bind 失败时
            // 操作者会先看到日志里的 token、却因启动失败而作废。签发不出来说明注册表
            // 写不动，那是与 bind 失败同级的问题——如实启动失败，不静默降级。
            match crate::node_link::runtime::issue_bootstrap_token(
                registry,
                cfg.node_link.bootstrap_token_ttl_secs,
            )
            .await
            {
                Ok(Some(token)) => tracing::warn!(
                    "节点链路已开放（{}）。bootstrap 配对 token（只显示这一次，{} 秒内有效）：{}",
                    served.listen,
                    cfg.node_link.bootstrap_token_ttl_secs,
                    token
                ),
                Ok(None) => tracing::info!("节点链路已开放（{}）", served.listen),
                Err(e) => {
                    return Err(crate::error::SebasError::Config(format!(
                        "节点链路首配 token 签发失败：{e}"
                    )));
                }
            }
            Some(served)
        }
    };

    // 3.1（D4）：ready 打点后移——通道已 bind、secret 已落盘，此刻发 ready
    // 才满足「ready ⟹ 已武装」。裸 core（无 watchdog）此步为 no-op。
    if let Some(ipc) = watchdog_ipc.as_mut() {
        send_watchdog_ready(ipc).await;
    }

    // fail-fast-on-startup-errors：到这里启动已成功（ready）。清除启动错误
    // 闩锁文件，让 channel 客户端不会把陈旧的启动失败报给已恢复的部署。
    crate::startup_failure::clear_env_summary_file();

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

    // fail-fast-on-startup-errors：到这里启动已成功（ready）。清除启动错误
    // 闩锁文件，让 channel 客户端不会把陈旧的启动失败报给已恢复的部署。
    crate::startup_failure::clear_env_summary_file();

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
    // watchdog's restart probes the path. D5：secret 文件**不**随优雅退出
    // 删除——socket 文件才是「core 死了」的权威信号，残留 secret 无害
    // （socket 不在时客户端根本走不到握手）。
    {
        let _ = armed_channel.shutdown.send(true);
        // 节点链路随之收摊：停止接受新连接（已接入的节点会按退避重连，
        // 这与「主控重启不影响远端执行」一致——节点侧的执行不因主控离开而终止）。
        if let Some(node_link) = &armed_node_link {
            node_link.close();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // persist-session-map 3.1：关停快照（`dump_json` → `[dispatch]
    // state_file`）已退休——映射在每次生命周期事件处按变更落库，关停顺序
    // 不再决定什么能活下来（session-lifecycle「Snapshot precedes shutdown
    // kill」）。关停路径不再写任何会话映射文件，也不再有独立映射文件
    // （state-store「No separate session-map file exists」）。

    // Signal all live sessions to cancel and reap their child processes.
    mgr.kill_all().await;
    Ok(())
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(format!(
        "{}{}",
        cfg.log.level,
        crate::config::LOG_FILTER_QUIET
    ))
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

/// CLI `--webui-host` 的 loopback 判定。与 watchdog `WebUiEndpoint::is_loopback`
/// 同语义：只认 IP 字面量（127.0.0.1/::1 = loopback）；域名（含 `localhost`）
/// 一律按非 loopback 走安全门——两条启动路径的判定必须一致。
fn webui_host_is_loopback(host: &str) -> bool {
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
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

/// card 配置的**引导回退**：TOML `[card]`。
///
/// retire-legacy-state-json 4.1：`settings.json` 已退休，这里不再有第二层文件
/// 读取——库里的 `card_config` 由调用点（`load_merged_card_config` 那段）先读，
/// 只有「库里没有 / 库不可用」才落到这里。回退目标是 TOML `[card]` 而**不是**
/// `CardConfig::default()`：后者会静默丢掉操作员写进配置文件的卡片偏好。
fn fallback_settings(cfg: &Config) -> sebas_dispatch::CardConfig {
    serde_json::from_value(serde_json::to_value(&cfg.card).expect("card config serializes"))
        .expect("card config round-trips between mirror shapes")
}

/// 在 watchdog 下运行时向父进程发送 ready 握手（Ready-only 协议）。
/// 控制命令一律走 control RPC（Unix socket），pipe 不再承载命令。
/// 3.1（D4）：调用点已后移到核心通道 bind+武装之后。
async fn send_watchdog_ready(ipc: &mut crate::ipc::ChildIpc) {
    if let Err(e) = ipc.ready().await {
        tracing::warn!("failed to send watchdog IPC ready: {e}");
        return;
    }
    tracing::info!("watchdog IPC connected");
}

/// 自动武装核心会话通道（harden-core-channel-deployment 1.2，design D1/D3/D5）。
///
/// 时序即契约：先 bind（失败 → 启动失败，ready 永不发出），再解析并落盘
/// secret，最后 spawn accept 循环。secret 来源：`SEBAS_CORE_SECRET` env 优先
/// （watchdog 注入路径不变）；缺失时现场生成随机 secret。两种来源都原子
/// 写入 config 解析的 secret 文件（0600，unix），迟启动的客户端据此发现。
#[derive(Debug)]
pub(crate) struct ArmedChannel {
    /// 优雅关闭句柄（accept 循环退出后由 serve_bound 删除 socket 文件）。
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// secret 文件位置（测试断言 + 日志）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub secret_file: std::path::PathBuf,
    /// 实际生效的握手 secret（env 值或现场生成；测试断言文件内容一致）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub secret: String,
}

pub(crate) async fn arm_core_channel(
    cfg: &Config,
    config_path: &std::path::Path,
    backend: std::sync::Arc<dyn sebas_webui::session_backend::SessionBackend>,
    router: &sebas_dispatch::engine::DispatchHandle,
    // 节点链路的注册表写者句柄（add-remote-execution-node 2.7）：通道据此承载
    // 节点管理入口。`None` = 节点链路未启用（管理操作如实回 Disabled）。
    node_registry: Option<std::sync::Arc<tokio::sync::Mutex<crate::node_link::NodeRegistry>>>,
    // 远端会话投影（5.1）：`None` = 节点链路未启用。有了它，通道快照/订阅流里
    // 才会出现跑在节点上的会话；没有它就与今日完全一致。
    projection: Option<std::sync::Arc<crate::node_link::RemoteProjection>>,
) -> Result<ArmedChannel> {
    let channel_path = crate::core_channel::socket_path(cfg);
    // bind 先行（1.3/D4）：路径被存活进程占用 → 硬错误，调用方在 ready 之前
    // 拿到 Err，进程以 75 退出而不是顶着无通道继续跑。
    let listener =
        crate::core_channel::server::bind_channel_socket(&channel_path).map_err(|e| {
            crate::error::SebasError::Config(format!(
                "core session channel bind 失败 ({}): {e}",
                channel_path.display()
            ))
        })?;
    let (secret, secret_source) = match std::env::var("SEBAS_CORE_SECRET") {
        Ok(s) if !s.is_empty() => (s, "env"),
        _ => (crate::core_channel::generate_secret(), "generated"),
    };
    let secret_file = cfg.service.core.secret_file_path(config_path);
    crate::core_channel::write_secret_file(&secret_file, &secret).map_err(|e| {
        crate::error::SebasError::Config(format!(
            "core secret 文件写入失败 ({}): {e}",
            secret_file.display()
        ))
    })?;
    let (close_tx, close_rx) = tokio::sync::watch::channel(false);
    let serve_router = router.clone();
    let serve_secret = secret.clone();
    tokio::spawn(async move {
        match crate::core_channel::server::serve_bound(
            backend,
            serve_router,
            channel_path,
            serve_secret,
            listener,
            node_registry,
            projection,
            close_rx,
        )
        .await
        {
            Ok(()) => info!("core session channel closed"),
            Err(e) => warn!(?e, "core session channel server exited"),
        }
    });
    info!(
        path = %secret_file.display(),
        source = secret_source,
        "core session channel armed"
    );
    Ok(ArmedChannel {
        shutdown: close_tx,
        secret_file,
        secret,
    })
}

#[cfg(test)]
mod router_info_tests {
    use super::build_router_info;
    use sebas_router::config::{ProviderConfig, RouterConfig};
    use std::collections::HashMap;

    // fix-webui-detached-status 2.1：detached webui 与 in-process 共用同一
    // 装配。直接构造 RouterConfig，不走 env 敏感的 parse（并行测试会改
    // SEBAS_ROUTER_LISTEN，污染 parse）。
    fn gw_config() -> RouterConfig {
        RouterConfig {
            listen: "127.0.0.1:50770".into(),
            max_body_bytes: 1024,
            connect_timeout_secs: 5,
            read_timeout_secs: 5,
            usage_db: String::new(),
            usage_retention_days: 30,
            usage_max_rows: 200_000,
            usage_prune_interval_secs: 0,
            debug: false,
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
