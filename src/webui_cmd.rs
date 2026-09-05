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
    AdminAdapter, AdminEvent, AdminMutationResult, AdminOperation, AdminService, AdminStatus,
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
        warn!("webui 登录密码不足 8 个字符，强度较弱；公网部署建议换强密码");
    }

    auth::store_credentials(&path, &auth::Credentials::new(&username, &password))
        .map_err(|e| SebasError::Config(e))?;

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

/// webui 启动前的鉴权引导：
/// 1. 凭据文件缺失且 `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` 都在 →
///    自动建户（容器/公网部署）。
/// 2. 返回共享 [`AuthHandle`]。
pub fn bootstrap_auth() -> Arc<AuthHandle> {
    let path = auth::default_auth_file();
    if auth::load_credentials(&path).ok().flatten().is_none() {
        if let (Ok(user), Ok(pass)) = (
            std::env::var("SEBAS_WEBUI_USER"),
            std::env::var("SEBAS_WEBUI_PASSWORD"),
        ) {
            if !user.is_empty() && !pass.is_empty() {
                if pass.len() < 8 {
                    warn!("SEBAS_WEBUI_PASSWORD 不足 8 字符（测试环境约定 admin/admin 可接受），公网部署建议换强密码");
                }
                match auth::store_credentials(&path, &auth::Credentials::new(&user, &pass)) {
                    Ok(()) => info!("webui auth bootstrapped from env for user {user} ({})", path.display()),
                    Err(e) => warn!("webui auth bootstrap failed: {e}"),
                }
            }
        }
    }
    Arc::new(AuthHandle::open(path))
}

/// CLI entry: read + parse the config, then run the standalone WebUI server.
pub async fn run(args: WebUiArgs) -> Result<()> {
    init_tracing();

    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    // Build the WebUI endpoint from config (enabled, host, port).
    // Returns None when watchdog.webui.enabled is false — we require it to be
    // true because the standalone WebUI is a watchdog-owned service.
    let endpoint = WebUiEndpoint::from_config(&cfg.watchdog.webui)
        .ok_or_else(|| SebasError::Config("watchdog.webui.enabled is false".into()))?;

    // 登录鉴权：开关关闭（测试/联调）→ 注入 disabled 态，全路由免登录；
    // 开关打开（默认）→ 凭据文件缺失时可用 SEBAS_WEBUI_USER/SEBAS_WEBUI_PASSWORD
    // env 引导（容器部署），之后全部 /api 与 /ws 需要登录。
    let auth = if cfg.watchdog.webui.auth {
        bootstrap_auth()
    } else {
        warn!(
            "webui 鉴权已通过 [watchdog.webui] auth = false 关闭：全部路由免登录"
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
            "webui binds {}（非 loopback）：确认已配置登录凭据（{}）",
            endpoint.bind_addr(),
            auth.path().display()
        );
    }

    info!(
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
    let backend = crate::core_channel::client::CoreChannelBackend::new(
        crate::core_channel::socket_path(&cfg),
        std::env::var("SEBAS_CORE_SECRET").ok().unwrap_or_default(),
    );

    // Bind to the configured port. Fails if the port is already in use
    // (by another WebUI process or the legacy `sebas run --webui` path).
    // On failure, exit with a specific code so the watchdog supervisor can
    // distinguish bind failures from other crashes and mark the service as
    // Degraded instead of endlessly retrying.
    let listener = match tokio::net::TcpListener::bind(endpoint.bind_addr()).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                "bind webui {} failed: {e}; exiting with code {} (Degraded)",
                endpoint.bind_addr(),
                EXIT_BIND_FAILED,
            );
            std::process::exit(EXIT_BIND_FAILED);
        }
    };

    let admin_adapter = control_admin_adapter();

    info!("webui dashboard listening on {}", endpoint.bind_addr());

    // 创建会话下拉的可达 agent 列表（独立 WebUI 进程同样读 config 提供）。
    let agent_kinds: Vec<sebas_webui::agent_kinds::AgentKindSource> = cfg
        .acp
        .agents
        .keys()
        .map(|slug| sebas_webui::agent_kinds::AgentKindSource {
            slug: slug.clone(),
            command: cfg.acp.command_for(slug).unwrap_or_default(),
        })
        .collect();

    // Run the WebUI server. This blocks until the server stops.
    let backend_dyn: Arc<dyn sebas_webui::SessionBackend> = backend;
    sebas_webui::run_with_admin_adapter_and_auth(
        backend_dyn,
        sebas_webui::models::GatewayInfo::default(),
        merged_card_cfg,
        agent_kinds,
        listener,
        admin_adapter,
        auth,
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
            }) => Ok(AdminMutationResult {
                operation_id,
                status,
                message: message.into(),
            }),
            Ok(RpcControlResponse::Rejected { code, message }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }
}

#[async_trait]
impl AdminAdapter for ControlRpcAdminAdapter {
    async fn status(&self) -> std::result::Result<AdminStatus, String> {
        match self.send_request(RpcControlRequest::Status).await {
            Ok(RpcControlResponse::Accepted {
                operation_id,
                status,
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
            Ok(RpcControlResponse::Rejected { code, message }) => {
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
            Ok(RpcControlResponse::Rejected { code, message }) => {
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
    ) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::ServiceSet {
                service: service.into(),
                desired: desired.into(),
                // WebUI 服务页的启停选择持久化：watchdog 重启后保持用户意图。
                persist: true,
            },
            format!("service {service} set to {desired}"),
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
            Ok(RpcControlResponse::Rejected { code, message }) => {
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
    match sebas_router::settings::load_settings(&sebas_router::settings::settings_path()) {
        Ok(Some(s)) => serde_json::from_value(serde_json::to_value(&s).expect("card config serializes"))
            .expect("card config round-trips between mirror shapes"),
        Ok(None) => cfg.card.clone(),
        Err(e) => {
            warn!(error = %e, "settings.json parse failed; using config defaults");
            cfg.card.clone()
        }
    }
}

/// Install a tracing subscriber for the standalone WebUI process.
/// Filter comes from `RUST_LOG` (default `"info"`), mirroring gateway_cmd.
/// `try_init` is used so the first caller wins and later calls are no-ops.
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
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

[router]
state_file = "{dir}/state.json"

[media]
download_dir = "{dir}/media"

[acp.claude]
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
            .err()
            .expect("开关关闭 + 非 loopback 必须配置错误退出");
        let msg = err.to_string();
        assert!(msg.contains("非 loopback"), "{msg}");
    }

    #[tokio::test]
    async fn non_loopback_refused_when_switch_on_but_no_credentials() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = write_config(dir.path(), "0.0.0.0", true);
        let _auth_file = set_auth_file(dir.path());
        let err = run(WebUiArgs::new(config.to_string_lossy().into_owned()))
            .await
            .err()
            .expect("开关开 + 无凭据 + 非 loopback 必须配置错误退出");
        let msg = err.to_string();
        assert!(msg.contains("非 loopback"), "{msg}");
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
}
