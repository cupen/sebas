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
use sebas_webui::rbac::Role;
use sebas_webui::user_store::{StoreError, UserStore};
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
    /// 显式角色（root/admin/member/viewer）。缺省：库里第一个用户为 root，
    /// 其后为 member（design D7）。已存在用户时同时把其角色改为该值。
    pub role: Option<String>,
}

/// `sebas webui-passwd` — 初始化 / 修改 WebUI 登录用户（写 auth.db 用户库，
/// add-webui-multiuser-rbac 4.1，design D7）。
///
/// 改密 = 重跑同命令（写入新盐新哈希）；用户库即活数据，运行中的 webui
/// 进程每请求实时读库，改密即时生效、既有会话留待下次请求自然失效（或
/// 在 WebUI 用户管理里踢掉）。密码来源：`--password-stdin`（一行）或
/// `--password`。用户名必填（多用户库里没有「现有用户名」可沿用）；库里
/// 已有同名账户 → 改密（`--role` 同时改角色），否则建户——首个用户默认
/// root，其后默认 member，`--role` 显式覆盖。不再读写任何 JSON 凭据文件。
pub fn run_passwd(args: WebUiPasswdArgs) -> Result<()> {
    let path = auth::default_auth_db();
    let store = UserStore::open(&path)
        .map_err(|e| SebasError::Config(format!("打开 WebUI 用户库 {path:?} 失败: {e}")))?;

    let username = match args.user.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => {
            return Err(SebasError::Config(
                "缺少用户名：请用 --user <name> 指定要创建或改密的账户".into(),
            ))
        }
    };

    let explicit_role = args
        .role
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|word| {
            word.parse::<Role>().map_err(|e| {
                SebasError::Config(format!("--role 非法: {e}"))
            })
        })
        .transpose()?;

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
        // 不做硬性拦截：测试环境统一用 admin/admin 这类短密码；公网部署
        // 由部署者自己权衡强度（与首启 setup 页的 ≥8 硬门槛不同）。
        warn!("webui password is shorter than 8 chars, weak; use a strong password for public deploys");
    }

    let existing = store
        .get_by_username(&username)
        .map_err(|e| SebasError::Config(format!("查询用户库失败: {e}")))?;
    match existing {
        Some(user) => {
            store
                .set_password(user.id, &password)
                .map_err(|e| SebasError::Config(format!("改密失败: {e}")))?;
            if let Some(role) = explicit_role {
                store
                    .set_role(user.id, role)
                    .map_err(|e| SebasError::Config(format!("改角色失败: {e}")))?;
                println!(
                    "WebUI 用户已更新：用户 {}（角色 {}，{}）",
                    username,
                    role,
                    path.display()
                );
            } else {
                println!("WebUI 密码已更新：用户 {}（{}）", username, path.display());
            }
        }
        None => {
            // 首个用户默认 root，其后默认 member；`--role` 显式覆盖（design D7）。
            let role = explicit_role.unwrap_or(
                if store.count().unwrap_or(0) == 0 {
                    Role::Root
                } else {
                    Role::Member
                },
            );
            store
                .create(&username, &password, role)
                .map_err(|e| match e {
                    StoreError::UsernameTaken => {
                        SebasError::Config(format!("用户名 {username} 已存在"))
                    }
                    other => SebasError::Config(format!("建户失败: {other}")),
                })?;
            println!(
                "WebUI 登录用户已创建：用户 {}（角色 {}，{}）\n现在 webui 的全部 API/WebSocket 都需要登录；\
                 若需公网部署，把 [watchdog.webui] host 指到 0.0.0.0 即可。",
                username,
                role,
                path.display()
            );
        }
    }
    Ok(())
}

/// webui 启动前的鉴权引导（add-webui-multiuser-rbac 3.3，design D4 顺序）：
/// 1. 打开/初始化 auth.db（`SEBAS_WEBUI_AUTH_DB` 覆盖，默认 `~/.sebas/auth.db`）。
/// 2. 零用户且 `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` 非空 → 建 root
///    （容器/公网部署路径，spec「环境变量引导 root」；已有用户时静默跳过
///    ——幂等重启不吃错）。
/// 3. 仍零用户 → 保持零用户，loopback 下由 WebUI 首启设置页接手（
///    `POST /api/auth/setup`）；非 loopback 由 [`ensure_non_loopback_bind_allowed`]
///    在 bind 前拒启。
///
/// design D4 第 4 点的旧路径已整体删除，不做兼容：自动生成随机密码、旧
/// `webui-auth.json` 读取/迁移、`SEBAS_WEBUI_TOKEN`、`SEBAS_WEBUI_AUTH_FILE`
/// 均不再被读取或写入。
///
/// 返回共享 [`AuthHandle`]。env 引导失败只 warn（零用户库仍可用设置页
/// 兜底，不因引导失败拒启）。
pub fn bootstrap_auth() -> Arc<AuthHandle> {
    let path = auth::default_auth_db();
    let handle = Arc::new(AuthHandle::open(path.clone()));

    if let (Ok(user), Ok(pass)) = (
        std::env::var("SEBAS_WEBUI_USER"),
        std::env::var("SEBAS_WEBUI_PASSWORD"),
    ) && !user.trim().is_empty()
        && !pass.is_empty()
    {
        if pass.chars().count() < 8 {
            warn!("SEBAS_WEBUI_PASSWORD shorter than 8 chars (ok for test env admin/admin), use a strong password for public deploys");
        }
        match handle.user_store() {
            Some(store) => match store.setup_root(user.trim(), &pass) {
                Ok(_) => {
                    info!(
                        "webui root user {} bootstrapped from env ({})",
                        user.trim(),
                        path.display()
                    );
                }
                // 已有用户：env 引导让位（幂等重启不吃错）。
                Err(StoreError::AlreadyInitialized) => {}
                Err(e) => warn!("webui env bootstrap failed: {e}"),
            },
            None => warn!("webui auth.db unavailable, cannot bootstrap root from env"),
        }
    }
    handle
}

/// 用户库是否至少有一个启用用户。fail-closed：句柄不在场（disabled/库打不
/// 开）或读取失败一律视为没有——公网门不能对「鉴权不可用」开绿灯。
pub(crate) fn has_enabled_user(auth: &AuthHandle) -> bool {
    auth.user_store()
        .and_then(|store| store.list().ok())
        .map(|users| users.iter().any(|u| u.enabled))
        .unwrap_or(false)
}

/// 非 loopback bind 安全门（add-webui-multiuser-rbac 3.3，design D4 步骤 3；
/// spec「非 loopback bind 与开关联动」）。`sebas webui` 独立进程与
/// `core --webui` 内嵌两条启动路径共用同一裁决：
///
/// - 开关关闭：一律拒绝——误关开关叠加公网暴露只能是误配（spec 场景
///   「开关关闭拒绝公网 bind」）；
/// - 开关打开但用户库没有启用用户：拒绝。零用户 = 公网先访问者可抢注
///   root（spec 场景「开关打开但零用户拒绝公网 bind」）；全禁用 = 无人可
///   登录；库损坏 = 鉴权不可用——三者都不得绑公网。先经环境变量
///   （`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`）或
///   `sebas webui-passwd --user <name>` 建立 root 再绑公网。
///
/// （spec 原文按「存在至少一个启用用户」判定；比只看 `needs_setup()`
/// （= 零用户）更紧：全禁用/坏库同样拒绝，fail-closed。）
pub(crate) fn ensure_non_loopback_bind_allowed(
    auth_on: bool,
    auth: &AuthHandle,
) -> Result<()> {
    if !auth_on {
        return Err(SebasError::Config(
            "watchdog.webui.host 非 loopback：auth 开关必须保持打开（误关开关叠加公网暴露 = 误配）；\
             如需免鉴权请保持 loopback bind"
                .into(),
        ));
    }
    if !has_enabled_user(auth) {
        return Err(SebasError::Config(
            "watchdog.webui.host 非 loopback：用户库中还没有启用用户（零用户时公网先访问者可抢注 root）。\
             先用 SEBAS_WEBUI_USER + SEBAS_WEBUI_PASSWORD 环境变量或 \
             `sebas webui-passwd --user <name>` 建立 root，再绑非 loopback 地址"
                .into(),
        ));
    }
    Ok(())
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
    // 开关打开（默认）→ 按 design D4 引导（建库 → env 建 root → 零用户
    // 留给首启设置页），之后全部 /api 与 /ws 需要登录。
    let auth = if cfg.watchdog.webui.auth {
        bootstrap_auth()
    } else {
        warn!(
            "webui auth disabled via [watchdog.webui] auth = false: all routes are public"
        );
        Arc::new(AuthHandle::disabled())
    };

    // 非 loopback bind 安全门（add-webui-multiuser-rbac 3.3，design D4 步骤
    // 3；spec「非 loopback bind 与开关联动」）：开关关闭、或用户库没有启用
    // 用户（零用户设置页形态 / 全禁用 / 库损坏）都以配置错误退出，不绑定
    // 端口——没有登录门的控制面暴露公网等于裸奔，零用户公网 bind 更会让
    // 先访问者抢注 root。
    if !endpoint.is_loopback() {
        ensure_non_loopback_bind_allowed(cfg.watchdog.webui.auth, &auth)?;
        warn!(
            "webui binds {} (non-loopback): login auth enabled, user store at {}",
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
    //! add-webui-auth-switch：非 loopback 安全门与开关的联动（spec 场景），
    //! 以及 bootstrap_auth 的引导行为（add-webui-multiuser-rbac 3.3 收口：
    //! design D4 顺序——建库 → env 引导 root → 零用户留设置页/非 loopback
    //! 拒启；随机密码/token/旧 JSON 路径全部删除且被忽略）。

    use super::*;
    use sebas_webui::rbac::Role;

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

    struct AuthDbGuard(PathBuf);
    fn set_auth_db(dir: &std::path::Path) -> AuthDbGuard {
        let db = dir.join("auth.db");
        // SAFETY: ENV_LOCK 由调用方持有，无并发 env 访问。
        unsafe {
            std::env::set_var("SEBAS_WEBUI_AUTH_DB", &db);
        }
        AuthDbGuard(db)
    }
    impl Drop for AuthDbGuard {
        fn drop(&mut self) {
            // SAFETY: 同上，ENV_LOCK 仍被持有。
            unsafe {
                std::env::remove_var("SEBAS_WEBUI_AUTH_DB");
            }
        }
    }

    /// 把测试注入的全部引导 env 还原（AuthDbGuard 只管 AUTH_DB）。
    fn clear_bootstrap_env() {
        // SAFETY: ENV_LOCK 已被调用方持有，无并发 env 访问。
        unsafe {
            for name in [
                "SEBAS_WEBUI_USER",
                "SEBAS_WEBUI_PASSWORD",
                "SEBAS_WEBUI_AUTH_FILE",
                "SEBAS_WEBUI_TOKEN",
            ] {
                std::env::remove_var(name);
            }
        }
    }

    #[tokio::test]
    // env 锁有意横跨整个测试（含 await）：env 是进程全局的。
    #[allow(clippy::await_holding_lock)]
    async fn non_loopback_refused_when_switch_off() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(dir.path(), "0.0.0.0", false);
        let err = run(WebUiArgs::new(config.to_string_lossy().into_owned()))
            .await
            .expect_err("开关关闭 + 非 loopback 必须配置错误退出");
        let msg = err.to_string();
        assert!(msg.contains("非 loopback"), "{msg}");
    }

    /// spec 场景「开关打开但零用户拒绝公网 bind」（design D4 步骤 3）：
    /// auth 默认打开、用户库零用户（tempdir 沙箱库）→ 配置错误退出，不 bind。
    #[tokio::test]
    // env 锁有意横跨整个测试（含 await）。
    #[allow(clippy::await_holding_lock)]
    async fn non_loopback_refused_when_zero_users() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(dir.path(), "0.0.0.0", true);
        let _db = set_auth_db(dir.path());
        let err = run(WebUiArgs::new(config.to_string_lossy().into_owned()))
            .await
            .expect_err("零用户 + 非 loopback 必须配置错误退出");
        let msg = err.to_string();
        assert!(msg.contains("非 loopback"), "{msg}");
        assert!(
            msg.contains("启用用户"),
            "错误信息应指引用先建 root: {msg}"
        );
    }

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

    /// 安全门的三态判定（纯函数，不真 bind）：零用户拒、env 引导后放行、
    /// 开关关闭恒拒（spec 场景「开关打开且凭据存在允许公网 bind」）。
    #[test]
    fn gate_refuses_zero_users_then_allows_after_env_bootstrap() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let _db = set_auth_db(dir.path());
        clear_bootstrap_env();

        // 零用户：门拒绝（无论开关态——开关关闭走另一条拒绝分支）。
        let handle = bootstrap_auth();
        assert!(handle.needs_setup(), "沙箱库应保持零用户");
        assert!(ensure_non_loopback_bind_allowed(true, &handle).is_err());

        // env 引导 root 后：门放行（不真 bind——gate 是纯判定，serve 不在
        // 本测射程内）。
        // SAFETY: ENV_LOCK 已持有，无并发 env 访问。
        unsafe { std::env::set_var("SEBAS_WEBUI_USER", "admin") };
        unsafe { std::env::set_var("SEBAS_WEBUI_PASSWORD", "password8") };
        let handle = bootstrap_auth();
        assert!(!handle.needs_setup());
        ensure_non_loopback_bind_allowed(true, &handle)
            .expect("env 引导建立 root 后公网 bind 应放行");

        // 开关关闭：无论用户库为何都拒绝。
        assert!(ensure_non_loopback_bind_allowed(false, &handle).is_err());

        clear_bootstrap_env();
    }

    /// spec 场景「旧单账户凭据文件被忽略」：旧 `webui-auth.json` 存在、
    /// `SEBAS_WEBUI_AUTH_FILE` 与 `SEBAS_WEBUI_TOKEN` 都设置时，引导不读取
    /// 不迁移（用户库仍零用户进设置流程），旧文件原样保留。
    #[test]
    fn legacy_json_auth_file_and_token_are_ignored() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = set_auth_db(dir.path());
        clear_bootstrap_env();

        let legacy = dir.path().join("webui-auth.json");
        std::fs::write(
            &legacy,
            r#"{"username":"legacy-admin","password":"legacy-pw"}"#,
        )
        .unwrap();
        // SAFETY: ENV_LOCK 已持有，无并发 env 访问。
        unsafe { std::env::set_var("SEBAS_WEBUI_AUTH_FILE", &legacy) };
        unsafe { std::env::set_var("SEBAS_WEBUI_TOKEN", "legacy-token") };

        let handle = bootstrap_auth();
        assert!(handle.enabled());
        assert!(
            handle.needs_setup(),
            "旧凭据文件/token 在场时用户库仍必须为零用户（不读取不迁移）"
        );
        assert_eq!(handle.user_store().unwrap().count().unwrap(), 0);
        let raw = std::fs::read_to_string(&legacy).unwrap();
        assert!(
            raw.contains("legacy-admin"),
            "旧 JSON 不得被改写或删除: {raw}"
        );
        assert!(db.0.exists(), "auth.db 应按新路径创建");

        clear_bootstrap_env();
    }

    /// bootstrap_auth 的 env 全局性同样需要互斥（SEBAS_WEBUI_USER/
    /// SEBAS_WEBUI_PASSWORD/SEBAS_WEBUI_AUTH_DB 都是进程级）。
    #[test]
    fn bootstrap_opens_db_and_env_bootstraps_root() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = set_auth_db(dir.path());
        clear_bootstrap_env();

        // 1) 无任何 env → 建库并保持零用户（设置页形态；不再自动生成
        //    随机密码，spec「不自动生成任何默认凭据」）。
        let handle = bootstrap_auth();
        assert!(handle.enabled(), "开关打开时库句柄应视为启用");
        assert!(handle.needs_setup(), "零用户应是首启设置页形态");
        assert!(db.0.exists(), "auth.db 应被创建");

        // 2) USER/PASSWORD → 建 root（可登录）。
        // SAFETY: ENV_LOCK 已持有，无并发 env 访问。
        unsafe { std::env::set_var("SEBAS_WEBUI_USER", "admin") };
        unsafe { std::env::set_var("SEBAS_WEBUI_PASSWORD", "password8") };
        let handle = bootstrap_auth();
        assert!(!handle.needs_setup(), "env 引导后不再是设置页形态");
        let store = handle.user_store().expect("用户库在场");
        let root = store.get_by_username("admin").unwrap().expect("root 已建");
        assert_eq!(root.role, Role::Root);
        assert!(root.enabled);
        assert!(root.verify_password("password8"));

        // 3) 已有用户时重复 bootstrap 幂等：不建第二户、不覆盖。
        unsafe { std::env::set_var("SEBAS_WEBUI_PASSWORD", "other-pass-9") };
        let handle = bootstrap_auth();
        let store = handle.user_store().expect("用户库在场");
        assert_eq!(store.count().unwrap(), 1, "已有用户时 env 引导必须让位");
        assert!(
            store.get_by_username("admin").unwrap().unwrap().verify_password("password8"),
            "重复引导不得覆盖既有 root 的密码"
        );

        clear_bootstrap_env();
    }
}
