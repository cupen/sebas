//! Standalone WebUI server entry point.
//!
//! Spawned by the watchdog as a separate process when `[watchdog.webui] enabled`
//! is `true`. Runs independently of the core child so the dashboard stays up
//! across core restarts (`sebas watchdog` restarts the core child; the WebUI
//! process is unaffected).
//!
//! # Session data via the core session channel
//!
//! The standalone WebUI is a pure **client** of the core session channel (a
//! Unix-socket NDJSON protocol served by the core child): session reads and
//! mutations go through the socket backend (`core_channel::client`), so every
//! page shows the core's live state and every control reaches the real
//! session authority. When the core is not running, the backend reports
//! unreachable with its cause and the console renders that honestly —
//! no control reports success.

use crate::config::Config;
use crate::error::{Result, SebasError};
use crate::watchdog::control_rpc::{
    self, ControlEnvelope, RpcActor, RpcControlRequest, RpcControlResponse,
};
use crate::watchdog::services::WebUiEndpoint;
use crate::watchdog::EXIT_BIND_FAILED;
use async_trait::async_trait;
use sebas_webui::auth::{self, AuthHandle};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};
use sebas_webui::admin::{
    AdminActionError, AdminAdapter, AdminEvent, AdminMutationResult, AdminOperation, AdminService,
    AdminStatus,
};

/// Arguments for `sebas webui --config <path>`.
pub struct WebUiArgs {
    pub config: String,
}

impl WebUiArgs {
    pub fn new(config: String) -> Self {
        Self { config }
    }
}

/// `sebas webui-passwd` 参数（CLI 层经 `From` 转入，字段同 cli::WebUiPasswdArgs）。
pub struct WebUiPasswdArgs {
    pub user: Option<String>,
    pub password: Option<String>,
    pub password_stdin: bool,
}

/// `sebas webui-passwd` — 初始化 / 修改 WebUI 登录账户。
///
/// 改密 = 重跑同命令（写入新盐新哈希）；运行中的 webui 进程经 mtime
/// 热重载拾取，无需重启。密码来源：`--password-stdin`（一行）或
/// `--password`；用户名缺省沿用现有凭据。
pub fn run_passwd(args: WebUiPasswdArgs) -> Result<()> {
    let path = auth::default_auth_file();
    let existing = auth::load_credentials(&path)
        .map_err(|e| SebasError::Config(format!("webui 凭据文件损坏，请先删除 {path:?}: {e}")))?;

    let username = match args.user.or_else(|| existing.as_ref().map(|c| c.username.clone())) {
        Some(u) if !u.trim().is_empty() => u,
        _ => {
            return Err(SebasError::Config(
                "缺少用户名：首次建户请用 --user <name>（修改密码可省略，沿用现有用户名）"
                    .into(),
            ))
        }
    };

    let password = if args.password_stdin {
        use std::io::Read;
        let mut line = String::new();
        std::io::stdin()
            .read_to_string(&mut line)
            .map_err(|e| SebasError::Config(format!("read password from stdin: {e}")))?;
        // 去掉行尾换行（含 Windows CRLF）；其余字符原样参与哈希。
        line.trim_end_matches(['\r', '\n']).to_string()
    } else {
        args.password.ok_or_else(|| {
            SebasError::Config(
                "缺少密码：用 --password-stdin（推荐，避免进 shell history）或 --password"
                    .into(),
            )
        })?
    };
    if password.is_empty() {
        return Err(SebasError::Config("密码不能为空".into()));
    }
    if password.chars().count() < 8 {
        // 不做硬性拦截：测试环境统一用 admin/admin 这类短密码（见
        // scripts/test_webui_sandbox.sh）；公网部署由部署者自己权衡强度。
        warn!("webui password is shorter than 8 chars, weak; use a strong password for public deploys");
    }

    // 改密保留已注入的登录 token（token 由 env 引导管理，不经 passwd 覆写）。
    let mut credentials = auth::Credentials::new(&username, &password);
    if let Some(existing) = &existing {
        credentials.token_hash = existing.token_hash;
    }
    auth::store_credentials(&path, &credentials).map_err(SebasError::Config)?;

    match existing {
        Some(_) => println!("WebUI 密码已更新：用户 {}（{}）", username, path.display()),
        None => println!(
            "WebUI 登录账户已创建：用户 {}（{}）\n现在 webui 的全部 API/WebSocket 都需要登录；\
             若需公网部署，把 [watchdog.webui] host 指到 0.0.0.0 即可。",
            username,
            path.display()
        ),
    }
    Ok(())
}

/// webui 启动前的鉴权引导（开关打开 + 凭据文件缺失时，按优先级）：
/// 1. `SEBAS_WEBUI_TOKEN` → 注入单字段登录密钥（SHA-256 落盘）。
/// 2. `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` → 注入密码（容器/公网部署）。
/// 3. 两者皆缺 → 自动生成随机密码（默认开箱即要求登录；Jupyter 风格，
///    密码打印一次到日志，服务端只留 PBKDF2 哈希）。
///
/// 返回共享 [`AuthHandle`]。任何一步落盘失败都只 warn（登录门以文件实况
/// 为准，不因引导失败拒启）。
pub fn bootstrap_auth() -> Arc<AuthHandle> {
    const DEFAULT_BOOTSTRAP_USER: &str = "admin";
    let path = auth::default_auth_file();
    if auth::load_credentials(&path).ok().flatten().is_none() {
        let bootstrapped = if let Ok(token) = std::env::var("SEBAS_WEBUI_TOKEN")
            && !token.trim().is_empty()
        {
            let mut credentials = auth::Credentials::new(DEFAULT_BOOTSTRAP_USER, token.trim());
            use sha2::Digest;
            credentials.token_hash = Some(sha2::Sha256::digest(token.trim().as_bytes()).into());
            match auth::store_credentials(&path, &credentials) {
                Ok(()) => {
                    info!(
                        "webui auth bootstrapped from SEBAS_WEBUI_TOKEN ({})",
                        path.display()
                    );
                    true
                }
                Err(e) => {
                    warn!("webui auth bootstrap (token) failed: {e}");
                    false
                }
            }
        } else if let (Ok(user), Ok(pass)) = (
            std::env::var("SEBAS_WEBUI_USER"),
            std::env::var("SEBAS_WEBUI_PASSWORD"),
        ) && !user.is_empty()
            && !pass.is_empty()
        {
            if pass.chars().count() < 8 {
                warn!("SEBAS_WEBUI_PASSWORD shorter than 8 chars (ok for test env admin/admin), use a strong password for public deploys");
            }
            match auth::store_credentials(&path, &auth::Credentials::new(&user, &pass)) {
                Ok(()) => {
                    info!("webui auth bootstrapped from env for user {user} ({})", path.display());
                    true
                }
                Err(e) => {
                    warn!("webui auth bootstrap failed: {e}");
                    false
                }
            }
        } else {
            let password = auth::generate_random_password();
            match auth::store_credentials(
                &path,
                &auth::Credentials::new(DEFAULT_BOOTSTRAP_USER, &password),
            ) {
                Ok(()) => {
                    eprintln!(
                        "\n  webui 首次启动：已自动生成登录凭据（auth 默认开启）\n\n    用户名: {DEFAULT_BOOTSTRAP_USER}\n    密码:   {password}\n\n  已写入 {}，可用 `sebas webui-passwd` 修改，或设 SEBAS_WEBUI_TOKEN / SEBAS_WEBUI_PASSWORD 自带凭据。\n",
                        path.display()
                    );
                    warn!(
                        "auto-generated bootstrap credentials for user {DEFAULT_BOOTSTRAP_USER} written to {} (password printed once above)",
                        path.display()
                    );
                    true
                }
                Err(e) => {
                    warn!("webui auto-bootstrap failed: {e}");
                    false
                }
            }
        };
        if !bootstrapped {
            warn!("webui auth switch is on but no credentials could be provisioned; routes stay open until a credentials file appears");
        }
    }
    Arc::new(AuthHandle::open(path))
}

/// CLI entry: read + parse the config, then run the standalone WebUI server.
pub async fn run(args: WebUiArgs) -> Result<()> {
    init_tracing(None);

    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    // Build the WebUI endpoint from config (enabled, host, port).
    // Returns None when watchdog.webui.enabled is false — we require it to be
    // true because the standalone WebUI is a watchdog-owned service.
    let endpoint = WebUiEndpoint::from_config(&cfg.watchdog.webui)
        .ok_or_else(|| SebasError::Config("watchdog.webui.enabled is false".into()))?;

    // 登录鉴权：开关关闭（测试/联调）→ 注入 disabled 态，全路由免登录；
    // 开关打开（默认）→ 引导凭据（SEBAS_WEBUI_TOKEN → SEBAS_WEBUI_USER/
    // SEBAS_WEBUI_PASSWORD → 自动生成随机密码），之后全部 /api 与 /ws 需要登录。
    let auth = if cfg.watchdog.webui.auth {
        bootstrap_auth()
    } else {
        warn!(
            "webui auth disabled via [watchdog.webui] auth = false: all routes are public"
        );
        Arc::new(AuthHandle::disabled())
    };

    // 非 loopback bind（公网/局域网部署）只在「开关打开且凭据存在」时放行
    // ——没有登录门就把控制面暴露到公网等于裸奔；开关关闭即意图免鉴权，
    // 此时公网 bind 只能是误配，启动时硬失败。
    if !endpoint.is_loopback() && !(cfg.watchdog.webui.auth && auth.enabled()) {
        return Err(SebasError::Config(
            "watchdog.webui.host 非 loopback：必须先配置 WebUI 登录凭据 \
             （`sebas webui-passwd --user <name>` 或 SEBAS_WEBUI_USER/SEBAS_WEBUI_PASSWORD），\
             且 auth 保持打开"
                .into(),
        ));
    }
    if !endpoint.is_loopback() {
        warn!(
            "webui binds {} (non-loopback): make sure login credentials are set ({})",
            endpoint.bind_addr(),
            auth.path().display()
        );
    }

    tracing::debug!(
        "starting standalone webui on {} (config={})",
        endpoint.bind_addr(),
        args.config
    );

    // Load card config: settings.json wins if present, else TOML `[card]`.
    // (The session channel does not transport settings; the settings page
    // renders this local snapshot.)
    let merged_card_cfg = load_card_config(&cfg);

    // The session backend: a client of the core session channel. The core
    // child owns the sessions; this process only renders and forwards.
    // harden-core-channel-deployment（2.2/D2）：secret 不再是启动门槛——
    // env 优先，缺失时按同一份 config 发现 secret 文件（core 自动武装落盘）；
    // 两者皆缺省时 warn 一次并以空 secret 尝试（握手被拒 → reachability
    // 如实上报 `secret rejected`），不再 ready 前退出 75：旧 core（无自动
    // 武装）下行为同今天（socket absent），诚实性不变差，新装配则开箱即用。
    let secret_file = crate::config::core_secret_file_path(
        cfg.watchdog.core.secret_file.as_deref(),
        std::path::Path::new(&args.config),
    );
    let backend = crate::core_channel::client::CoreChannelBackend::with_secret(
        crate::core_channel::socket_path(&cfg),
        crate::core_channel::secret::ChannelSecret::from_env_or_file(Some(secret_file)),
    );

    // Bind to the configured port. Fails if the port is already in use
    // (by another WebUI process or the legacy `sebas core --webui` path).
    // On failure, exit with a specific code so the watchdog supervisor can
    // distinguish bind failures from other crashes and mark the service as
    // Degraded instead of endlessly retrying. fail-fast-on-startup-errors：
    // 退出码仍为 75（= EXIT_STARTUP_FAILURE），但统一走 startup-failure 摘要
    // 出口（stderr 末行 + SEBAS_STARTUP_ERROR_FILE，任务 1.2/1.3）。
    let listener = match tokio::net::TcpListener::bind(endpoint.bind_addr()).await {
        Ok(l) => l,
        Err(e) => {
            let cause = format!(
                "webui 端口绑定失败 {}（端口被占用？）: {e}",
                endpoint.bind_addr()
            );
            warn!("{cause}; exiting with code {EXIT_BIND_FAILED} (Degraded)");
            crate::startup_failure::exit_startup_failure(&cause);
        }
    };

    let admin_adapter = control_admin_adapter();

    // 监听就绪由 sebas_webui::server 统一记录（embedded 模式只有那一条）。
    // 创建会话下拉的可达 agent 列表（独立 WebUI 进程同样读 config 提供）。
    // 排序确定性：同 run.rs（default kind 最先，其余字典序）。
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

    // add-webui-picker-workdir-start：browse-dirs 的服务端默认浏览根，取值
    // 与 core --webui 内嵌形态一致（默认 kind 的 work_dir，回退进程 cwd）。
    let webui_work_root = match cfg.acp.work_dir_for(cfg.acp.default_kind()) {
        Some(dir) => Some(std::path::PathBuf::from(dir)),
        None => std::env::current_dir().ok(),
    };

    // add-webui-allowed-roots：白名单由纯函数组装（与 core --webui 内嵌
    // 形态同一语义：未配置 = 空表不启用；配置了 = 默认根自动入列）。
    let webui_allowed_roots =
        crate::config::webui_allowed_roots(&cfg.watchdog.webui, webui_work_root.as_deref());

    // Run the WebUI server. This blocks until the server stops.
    // fix-webui-detached-status：router 静态事实（listen/debug/has_auth 与
    // TOML 声明的 provider）与 in-process 形态同一装配，不再以
    // `RouterInfo::default()` 占位——那会让 composer 恒显
    // "no provider configured"。
    let router_info = crate::run::build_router_info(
        sebas_router::config::RouterConfig::parse(&raw).ok().as_ref(),
    );
    let backend_dyn: Arc<dyn sebas_webui::SessionBackend> = backend;
    sebas_webui::run_with_admin_adapter_and_auth(
        backend_dyn,
        router_info,
        merged_card_cfg,
        agent_kinds,
        listener,
        admin_adapter,
        auth,
        webui_work_root,
        webui_allowed_roots,
        cfg.watchdog.webui.archive_retention_days,
    )
    .await;

    info!("webui dashboard stopped");
    Ok(())
}

fn control_admin_adapter() -> Option<Arc<dyn AdminAdapter>> {
    let secret = match std::env::var("SEBAS_CONTROL_SECRET") {
        Ok(secret) if !secret.is_empty() => secret,
        _ => {
            warn!("SEBAS_CONTROL_SECRET not set; admin control routes are read-only");
            return None;
        }
    };
    Some(Arc::new(ControlRpcAdminAdapter {
        socket_path: control_rpc::default_socket_path(),
        secret,
    }))
}

struct ControlRpcAdminAdapter {
    socket_path: PathBuf,
    secret: String,
}

impl ControlRpcAdminAdapter {
    async fn send_request(&self, request: RpcControlRequest) -> Result<RpcControlResponse> {
        control_rpc::request(
            &self.socket_path,
            &ControlEnvelope {
                version: 1,
                request_id: "webui_admin".into(),
                secret: self.secret.clone(),
                actor: RpcActor::Cli { uid: current_uid() },
                request,
            },
        )
        .await
    }

    async fn submit(
        &self,
        request: RpcControlRequest,
        message: impl Into<String>,
    ) -> std::result::Result<AdminMutationResult, String> {
        match self.send_request(request).await {
            Ok(RpcControlResponse::Accepted {
                operation_id,
                status,
                ..
            }) => Ok(AdminMutationResult {
                operation_id,
                status,
                message: message.into(),
            }),
            Ok(RpcControlResponse::Rejected {
                code, message, ..
            }) => Err(format!("rejected [{code}]: {message}")),
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }
}

/// `ServiceSet` 的 RPC 应答 → 适配器结果（unify-router-process-shape 2.3）。
/// 纯函数：便于对拒绝载荷（code + count）的映射做无 socket 单测。
/// - `Rejected { code: active_routed_sessions, count }` → 结构化
///   [`AdminActionError`]（webui BFF 据此回 400 + 顶层 code + count）；
/// - 其余拒绝 → 原样保留 code/message（HTTP 形态与既有 500 一致）。
fn service_set_response(
    response: Result<RpcControlResponse>,
    service: &str,
    desired: &str,
) -> std::result::Result<AdminMutationResult, AdminActionError> {
    match response {
        Ok(RpcControlResponse::Accepted {
            operation_id,
            status,
            ..
        }) => Ok(AdminMutationResult {
            operation_id,
            status,
            message: format!("service {service} set to {desired}"),
        }),
        Ok(RpcControlResponse::Rejected {
            code,
            message,
            count,
        }) => Err(AdminActionError { code, message, count }),
        Ok(other) => Err(AdminActionError::other(format!(
            "unexpected response: {other:?}"
        ))),
        Err(e) => Err(AdminActionError::other(format!("control RPC failed: {e}"))),
    }
}

#[async_trait]
impl AdminAdapter for ControlRpcAdminAdapter {
    async fn status(&self) -> std::result::Result<AdminStatus, String> {
        match self.send_request(RpcControlRequest::Status).await {
            Ok(RpcControlResponse::Accepted {
                operation_id,
                status,
                ..
            }) => {
                let operation = AdminOperation {
                    operation_id,
                    request_type: "status".into(),
                    status,
                    message: "control RPC connected".into(),
                };
                Ok(AdminStatus {
                    version: env!("CARGO_PKG_VERSION").into(),
                    uptime_secs: 0,
                    operations: vec![operation.clone()],
                    active_operation: Some(operation),
                })
            }
            Ok(RpcControlResponse::Rejected { code, message, .. }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }

    async fn events_since(&self, seq: u64) -> std::result::Result<Vec<AdminEvent>, String> {
        match self
            .send_request(RpcControlRequest::EventsSince { seq })
            .await
        {
            Ok(RpcControlResponse::Events { events }) => Ok(events
                .into_iter()
                .map(|e| AdminEvent {
                    seq: e.seq,
                    operation_id: e.operation_id,
                    kind: e.kind,
                    message: e.public_message,
                })
                .collect()),
            Ok(RpcControlResponse::Rejected { code, message, .. }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }

    async fn service_set(
        &self,
        service: &str,
        desired: &str,
        force: bool,
    ) -> std::result::Result<AdminMutationResult, AdminActionError> {
        let response = self
            .send_request(RpcControlRequest::ServiceSet {
                service: service.into(),
                desired: desired.into(),
                // WebUI 服务页的启停选择持久化：watchdog 重启后保持用户意图。
                persist: true,
                // unify-router-process-shape 2.3：强制出口流的 force 透传
                // （watchdog executor 端执法，非 router-stop 组合忽略）。
                force,
            })
            .await;
        service_set_response(response, service, desired)
    }

    async fn service_restart(&self, service: &str) -> std::result::Result<AdminMutationResult, String> {
        // 走 watchdog 监督循环既有的 ServiceRestart（/router restart 同源）；
        // core 在 RPC 层被拒（升级/回滚语义归 RestartCore），前端对 core
        // 直接走 /api/admin/restart，不会发到这里。
        self.submit(
            RpcControlRequest::ServiceRestart {
                service: service.into(),
            },
            format!("service {service} restart accepted"),
        )
        .await
    }

    async fn update(
        &self,
        dev: bool,
        dry_run: bool,
    ) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::Update { dev, dry_run },
            format!("update accepted (dev={dev}, dry_run={dry_run})"),
        )
        .await
    }

    async fn rollback(&self, dry_run: bool) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::Rollback { dry_run },
            format!("rollback accepted (dry_run={dry_run})"),
        )
        .await
    }

    async fn restart_core(&self) -> std::result::Result<AdminMutationResult, String> {
        self.submit(RpcControlRequest::RestartCore, "restart core accepted")
            .await
    }

    async fn services(&self) -> std::result::Result<Vec<AdminService>, String> {
        match self.send_request(RpcControlRequest::ServiceStatus).await {
            Ok(RpcControlResponse::Services { services }) => Ok(services
                .into_iter()
                .map(|s| AdminService {
                    name: s.name,
                    status: s.status,
                    desired: s.desired,
                    uptime_secs: s.uptime_secs,
                })
                .collect()),
            Ok(RpcControlResponse::Rejected { code, message, .. }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Load card config from settings.json, falling back to the TOML `[card]` section.
fn load_card_config(cfg: &Config) -> sebas_feishu::cards::CardConfig {
    match sebas_dispatch::settings::load_settings(&sebas_dispatch::settings::settings_path()) {
        Ok(Some(s)) => serde_json::from_value(serde_json::to_value(&s).expect("card config serializes"))
            .expect("card config round-trips between mirror shapes"),
        Ok(None) => cfg.card.clone(),
        Err(e) => {
            warn!(error = %e, "settings.json parse failed; using config defaults");
            cfg.card.clone()
        }
    }
}

#[cfg(test)]
mod service_set_tests {
    //! unify-router-process-shape 2.3：webui admin adapter 对 force 透传与
    //! router 停止保护拒绝载荷的映射（纯函数，无需真实 control RPC socket）。

    use super::*;
    use sebas_webui::admin::ACTIVE_ROUTED_SESSIONS_CODE;

    #[test]
    fn rejection_with_active_routed_sessions_keeps_code_and_count() {
        let resp = Ok(RpcControlResponse::Rejected {
            code: ACTIVE_ROUTED_SESSIONS_CODE.into(),
            message: "router 有 3 个活跃 routed 会话".into(),
            count: Some(3),
        });
        let err = service_set_response(resp, "router", "off")
            .expect_err("保护拒绝必须映射为错误");
        assert_eq!(err.code, "active_routed_sessions");
        assert_eq!(err.count, Some(3));
        assert!(err.is_active_routed_sessions(), "前端据此弹强制出口");
        // wire 形状钉死：序列化后 code/count 字段名不变（count 在场）。
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "active_routed_sessions");
        assert_eq!(json["count"], 3);
    }

    #[test]
    fn other_rejections_map_without_count() {
        let resp = Ok(RpcControlResponse::Rejected {
            code: "invalid_request".into(),
            message: "未知服务: nope".into(),
            count: None,
        });
        let err =
            service_set_response(resp, "nope", "off").expect_err("非法服务必须映射为错误");
        assert_eq!(err.code, "invalid_request");
        assert_eq!(err.count, None);
        // count 缺席时 wire 上省略字段（旧形状不变）。
        let json = serde_json::to_value(&err).unwrap();
        assert!(json.get("count").is_none(), "{json}");
    }

    #[test]
    fn accepted_maps_to_mutation_result() {
        let resp = Ok(RpcControlResponse::Accepted {
            operation_id: "op_1".into(),
            status: "Accepted".into(),
            startup_failure: None,
        });
        let out = service_set_response(resp, "router", "off").expect("接受必须映射为结果");
        assert_eq!(out.operation_id, "op_1");
        assert_eq!(out.message, "service router set to off");
    }

    /// RPC 传输失败（watchdog 不在等）：internal 错误码，无 count。
    #[test]
    fn transport_failure_maps_to_internal_error() {
        let resp: Result<RpcControlResponse> = Err(SebasError::Upgrade("socket gone".into()));
        let err = service_set_response(resp, "router", "off").expect_err("必须失败");
        assert_eq!(err.code, "internal");
        assert_eq!(err.count, None);
    }
}

/// Install a tracing subscriber for the standalone WebUI process.
/// Filter comes from `RUST_LOG` (default `"info"`), mirroring router_cmd.
/// `try_init` is used so the first caller wins and later calls are no-ops.
pub fn init_tracing_for_im(log_filter: Option<&str>) {
    init_tracing(log_filter);
}

fn init_tracing(log_filter: Option<&str>) {
    use tracing_subscriber::{EnvFilter, fmt};
    // `--log-level` 显式给出时优先于 RUST_LOG；两者皆缺省退默认 info。
    let filter = match log_filter {
        Some(f) => EnvFilter::try_new(format!("{f}{}", crate::config::LOG_FILTER_QUIET))
            .unwrap_or_else(|_| EnvFilter::new(crate::config::DEFAULT_LOG_FILTER)),
        None => EnvFilter::try_from_env("RUST_LOG")
            .unwrap_or_else(|_| EnvFilter::new(crate::config::DEFAULT_LOG_FILTER)),
    };
    let _ = fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod auth_gate_tests {
    //! add-webui-auth-switch：非 loopback 安全门与开关的联动（spec 场景）。

    use super::*;
    use sebas_webui::auth::{self, Credentials};

    /// 写一份沙箱配置并返回 config 路径。host/auth 按用例注入。
    fn write_config(dir: &std::path::Path, host: &str, auth: bool) -> PathBuf {
        let path = dir.join("config.toml");
        // TOML basic string 里反斜杠是转义前缀（Windows 路径必炸），统一正斜杠。
        let dir = dir.display().to_string().replace('\\', "/");
        std::fs::write(
            &path,
            format!(
                r#"[feishu]
app_id = ""
app_secret = ""

[dispatch]
state_file = "{dir}/state.json"

[media]
download_dir = "{dir}/media"

[acp.agents.claude]
driver = "claude"
path = "claude"
args = []

[watchdog.core]
enabled = false
channel_path = "{dir}/core.sock"

[watchdog.webui]
enabled = true
host = "{host}"
port = 9879
auth = {auth}
"#,
            ),
        )
        .unwrap();
        path
    }

    /// env var 是进程全局的：并行用例共用会互相污染（A 的凭据路径泄给 B，
    /// 甚至让 B 绕过 gate 真的去 bind 0.0.0.0:9879），用互斥锁串行化。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct AuthFileGuard;
    fn set_auth_file(dir: &std::path::Path) -> AuthFileGuard {
        // SAFETY: ENV_LOCK 由调用方持有，无并发 env 访问。
        unsafe {
            std::env::set_var("SEBAS_WEBUI_AUTH_FILE", dir.join("webui-auth.json"));
        }
        AuthFileGuard
    }
    impl Drop for AuthFileGuard {
        fn drop(&mut self) {
            // SAFETY: 同上，ENV_LOCK 仍被持有。
            unsafe {
                std::env::remove_var("SEBAS_WEBUI_AUTH_FILE");
            }
        }
    }

    #[tokio::test]
    // env 锁有意横跨整个测试（含 await）：env 是进程全局的。
    #[allow(clippy::await_holding_lock)]
    async fn non_loopback_refused_when_switch_off_even_with_credentials() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(dir.path(), "0.0.0.0", false);
        // 凭据存在：拒绝理由必须来自开关关闭，而不是缺凭据。
        auth::store_credentials(
            &dir.path().join("webui-auth.json"),
            &Credentials::new("admin", "admin-admin"),
        )
        .unwrap();
        let _auth_file = set_auth_file(dir.path());
        let err = run(WebUiArgs::new(config.to_string_lossy().into_owned()))
            .await
            .expect_err("开关关闭 + 非 loopback 必须配置错误退出");
        let msg = err.to_string();
        assert!(msg.contains("非 loopback"), "{msg}");
    }

    // 「开关打开 + 凭据缺失 + 非 loopback」的旧拒绝场景已随自动引导移除：
    // bootstrap_auth 现在总能让凭据存在（token env → 密码 env → 随机生成），
    // 该状态下启动放行而非退出；引导行为本身由
    // `bootstrap_prefers_token_then_password_then_random` 覆盖。

    #[test]
    fn loopback_starts_with_switch_off() {
        // 开关关 + loopback：免鉴权启动是合法形态。run() 会阻塞在 serve 上，
        // 故只验证 gate 之前的路径——用无效 SEBAS_CORE_SECRET 不影响；
        // 这里退一步只断言「不再因鉴权门报配置错误」：直接检查解析层 +
        // endpoint 构造，避免拉起常驻进程。
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(dir.path(), "127.0.0.1", false);
        let raw = std::fs::read_to_string(&config).unwrap();
        let cfg = crate::config::Config::parse(&raw).unwrap();
        assert!(!cfg.watchdog.webui.auth);
        assert!(WebUiEndpoint::from_config(&cfg.watchdog.webui).unwrap().is_loopback());
    }

    /// bootstrap_auth 的 env 全局性同样需要互斥（SEBAS_WEBUI_TOKEN 与
    /// SEBAS_WEBUI_AUTH_FILE 都是进程级）。
    #[test]
    // env 锁横跨整个测试体。
    #[allow(clippy::await_holding_lock)]
    fn bootstrap_prefers_token_then_password_then_random() {
        use sebas_webui::auth::load_credentials;

        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let _auth_file = set_auth_file(dir.path());

        // 1) 无任何 env → 自动生成：凭据文件出现，admin 可登录。
        let handle = bootstrap_auth();
        assert!(handle.enabled(), "无凭据时应自动生成而不是保持关闭");
        assert_eq!(handle.username().as_deref(), Some("admin"));
        std::fs::remove_file(dir.path().join("webui-auth.json")).unwrap();

        // 2) 仅 TOKEN → verify_secret 命中 token。
        // SAFETY: ENV_LOCK 已持有，无并发 env 访问。
        unsafe { std::env::set_var("SEBAS_WEBUI_TOKEN", "tok-abc-123456") };
        let handle = bootstrap_auth();
        assert!(handle.enabled());
        let credentials = load_credentials(&dir.path().join("webui-auth.json"))
            .unwrap()
            .unwrap();
        assert!(credentials.verify_secret("tok-abc-123456"));
        assert!(!credentials.verify_secret("other"));
        // 顺手断言热重载后 login_secret 也通（与落盘凭据同一句柄语义）。
        std::fs::remove_file(dir.path().join("webui-auth.json")).unwrap();

        // 3) USER/PASSWORD → 密码注入；TOKEN 存在时 TOKEN 优先（空值除外）。
        unsafe { std::env::set_var("SEBAS_WEBUI_TOKEN", "") };
        unsafe { std::env::set_var("SEBAS_WEBUI_USER", "admin") };
        unsafe { std::env::set_var("SEBAS_WEBUI_PASSWORD", "password8") };
        let handle = bootstrap_auth();
        let credentials = load_credentials(&dir.path().join("webui-auth.json"))
            .unwrap()
            .unwrap();
        assert!(credentials.verify("admin", "password8"));
        assert!(credentials.token_hash.is_none(), "纯密码引导不写 token");
        drop(handle);

        // 清理本测注入的全部 env。
        unsafe { std::env::remove_var("SEBAS_WEBUI_TOKEN") };
        unsafe { std::env::remove_var("SEBAS_WEBUI_USER") };
        unsafe { std::env::remove_var("SEBAS_WEBUI_PASSWORD") };
    }
}
