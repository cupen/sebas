//! `sebas im` — 独立 IM 服务进程（extract-im-service 3.x）。
//!
//! webui 的兄弟服务：本进程不持有会话状态、不 spawn 任何 agent 子进程；
//! 全部会话操作经核心会话通道（`CoreChannelBackend`）请求核心，控制命令
//! 经 control RPC 直发 watchdog。feishu 适配器在本进程内运行（sebas-im
//! bootstrap 装配），入站事件由 im 前端蒸馏为端口请求。

use crate::config::Config;
use crate::error::{Result, SebasError};
use sebas_im::frontend::ImFrontend;
use sebas_im::port::{ControlPort, ControlRequest, CoreSessionPort};
use sebas_channels::ChannelKey;
use sebas_dispatch::{SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice, SessionBackend};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// `sebas im` 参数（镜像 `sebas webui`；`--test-msg`/`--dump-inbound` 自
/// core 随迁——它们是飞书接入面的启动行为，不属于核心）。
pub struct ImArgs {
    pub config: String,
    pub test_msg: Option<String>,
    pub dump_inbound: Option<String>,
}

/// 通道端口：把根 crate 的 `CoreChannelBackend` 适配成 im 的 `CoreSessionPort`。
struct ChannelPort {
    backend: Arc<crate::core_channel::client::CoreChannelBackend>,
}

#[async_trait::async_trait]
impl CoreSessionPort for ChannelPort {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        self.backend.snapshot().await
    }
    async fn ensure_message(
        &self,
        key: ChannelKey,
        message: String,
        attachments: Vec<sebas_im::port::ImAttachment>,
    ) -> std::result::Result<(), String> {
        let attachments = attachments
            .into_iter()
            .map(|a| crate::core_channel::protocol::Attachment {
                path: a.path,
                mime: a.mime,
                name: a.name,
            })
            .collect();
        self.backend
            .ensure_message_with(key, message, attachments)
            .await
            .map_err(|r| format!("{r:?}"))
    }
    async fn close(&self, key: ChannelKey) -> std::result::Result<(), String> {
        self.backend.close(key).await.map_err(|r| format!("{r:?}"))
    }
    async fn cancel(&self, key: ChannelKey) -> std::result::Result<(), String> {
        self.backend.cancel(key).await.map_err(|r| format!("{r:?}"))
    }
    async fn turns(&self, key: &ChannelKey, from: u64) -> Option<Vec<TurnEntry>> {
        self.backend.turns(key.clone(), from).await.ok()
    }
    fn subscribe_sessions(&self) -> broadcast::Receiver<SessionEvent> {
        self.backend.subscribe()
    }
    fn subscribe_approvals(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        self.backend.permission_requests()
    }
    async fn approval_answer(&self, request_id: &str, decision: PermissionDecision) -> bool {
        self.backend.answer_permission(request_id, decision).await
    }
    async fn state_snapshot(&self, domain: &str) -> Option<Value> {
        self.backend.state_snapshot(domain).await
    }
    async fn state_mutate(&self, domain: &str, payload: Value) -> std::result::Result<(), String> {
        self.backend.state_mutate(domain, payload).await
    }
}

/// watchdog 控制端口：控制命令直发 control RPC（不绕道 core）。
struct WatchdogControl {
    socket_path: PathBuf,
    secret: String,
}

impl WatchdogControl {
    async fn send(&self, request: crate::watchdog::control_rpc::RpcControlRequest) -> std::result::Result<String, String> {
        use crate::watchdog::control_rpc::{request as rpc_request, ControlEnvelope, RpcActor, RpcControlResponse};
        let envelope = ControlEnvelope {
            version: 1,
            request_id: "im_control".into(),
            secret: self.secret.clone(),
            actor: RpcActor::Cli { uid: uid_or_zero() },
            request,
        };
        match rpc_request(&self.socket_path, &envelope).await {
            Ok(resp) => match resp {
                RpcControlResponse::Accepted { operation_id, status, .. } => {
                    Ok(format!("已受理（op={operation_id}，状态 {status}）。"))
                }
                RpcControlResponse::Rejected { code, message } => {
                    Err(format!("watchdog 拒绝 [{code}]: {message}"))
                }
                other => Ok(format!("{other:?}")),
            },
            Err(e) => Err(format!("watchdog control RPC 失败：{e}")),
        }
    }
}

fn uid_or_zero() -> u32 {
    // 非 unix 平台没有 getuid；control RPC 的 peer-uid 校验在 unix 上生效。
    #[cfg(unix)]
    {
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[async_trait::async_trait]
impl ControlPort for WatchdogControl {
    async fn submit(&self, req: ControlRequest) -> std::result::Result<String, String> {
        use crate::watchdog::control_rpc::RpcControlRequest as R;
        match req {
            ControlRequest::Upgrade { dev, dry_run } => {
                self.send(R::Update { dev, dry_run }).await
            }
            ControlRequest::Rollback => self.send(R::Rollback { dry_run: false }).await,
            ControlRequest::Restart => self.send(R::RestartCore).await,
            ControlRequest::Services => self.send(R::ServiceStatus).await,
            ControlRequest::System => self.send(R::Status).await,
            ControlRequest::Router(args) => {
                let action = args.get("action").cloned().unwrap_or_default();
                match action.as_str() {
                    "status" => self.send(R::ServiceStatusFor { service: "router".into() }).await,
                    "on" | "off" => self
                        .send(R::ServiceSet {
                            service: "router".into(),
                            desired: action,
                            persist: false,
                        })
                        .await,
                    "restart" => self.send(R::ServiceRestart { service: "router".into() }).await,
                    other => Err(format!("/router 未知动作：{other}（on|off|restart|status）")),
                }
            }
            ControlRequest::Webui => {
                self.send(R::ServiceStatusFor { service: "webui".into() }).await
            }
            ControlRequest::Confirm { token } => self.send(R::Confirm { token }).await,
        }
    }
}

pub async fn run(args: ImArgs) -> Result<()> {
    crate::webui_cmd::init_tracing_for_im();

    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    if !cfg.feishu.is_enabled() {
        return Err(SebasError::Config(
            "feishu 未启用（[feishu] enabled / 凭据缺失）：im 服务没有可接入的 IM 通道".into(),
        ));
    }

    let core_secret = std::env::var("SEBAS_CORE_SECRET").unwrap_or_default();
    if core_secret.is_empty() {
        warn!("SEBAS_CORE_SECRET 未注入：无法连接核心会话通道（core 不可达时将如实降级）");
    }
    let control_secret = std::env::var("SEBAS_CONTROL_SECRET").unwrap_or_default();
    if control_secret.is_empty() {
        warn!("SEBAS_CONTROL_SECRET 未注入：控制命令（/upgrade 等）将以纯文本如实提示不可用");
    }

    // ws dump 目录（--dump-inbound 随迁自 core）。
    let dump_dir = match &args.dump_inbound {
        Some(p) => match std::fs::create_dir_all(p) {
            Ok(()) => Some(PathBuf::from(p)),
            Err(e) => {
                warn!(?e, "failed to create inbound dump dir; disabling dump");
                None
            }
        },
        None => None,
    };

    // 飞书装配（token 引导/问候/测试消息/adapter 实例化）——与 core 时代
    // 同一入口（sebas_im::bootstrap）。
    let card_cfg = load_card_config(&cfg);
    let boot = sebas_im::bootstrap::bootstrap(
        sebas_im::bootstrap::FeishuBootstrapConfig {
            app_id: cfg.feishu.app_id.clone(),
            app_secret: cfg.feishu.app_secret.clone(),
            owner_id: cfg.feishu.owner_id.clone(),
            allowed_chat_types: cfg.feishu.allowed_chat_types.clone(),
            bot_name: cfg.feishu.bot_name.clone(),
            hello_msg: cfg.feishu.hello_msg.clone(),
            card_config: card_cfg.clone(),
            dump_dir,
            channel_buffer: cfg.dispatch.channel_buffer,
        },
        args.test_msg,
    )
    .await
    .map_err(SebasError::Feishu)?;

    // 会话端口 + 控制端口 + 前端。
    let port = ChannelPort {
        backend: crate::core_channel::client::CoreChannelBackend::new(
            crate::core_channel::socket_path(&cfg),
            core_secret,
        ),
    };
    let control = WatchdogControl {
        socket_path: crate::watchdog::control_rpc::default_socket_path(),
        secret: control_secret,
    };
    let inbound = boot.spawn_inbound(cfg.dispatch.channel_buffer);
    let media_dir = shellexpand_dir(&cfg.media.download_dir);
    let frontend = Arc::new(ImFrontend::new(
        Arc::new(port),
        Arc::new(control),
        boot.client.clone(),
        boot.http.clone(),
        boot.tokens,
        card_cfg.theme_color.clone(),
        media_dir,
        cfg.media.max_file_size,
    ));

    info!("sebas im started; connecting to core session channel");
    if let Some(inbound) = inbound {
        frontend.run(inbound).await;
    } else {
        // WS spawn 失败：没有入站通道，进程按 core 时代语义继续等待关闭
        // 信号（watchdog 会按需重启）。
        info!("im service running without feishu inbound; idling");
        tokio::signal::ctrl_c().await.ok();
    }
    Ok(())
}

fn load_card_config(cfg: &Config) -> sebas_feishu::cards::CardConfig {
    // settings.json 若存在则整体优先（与 core 时代的 fallback_settings 同
    // 口径）；im 侧只读快照，持久化经通道状态库。
    match sebas_dispatch::settings::load_settings(&sebas_dispatch::settings::settings_path()) {
        Ok(Some(s)) => {
            // settings.json 是 router 中立 CardConfig 形状；转回 feishu 镜像
            // （与 core 时代 webui_card_cfg 的 serde 往返同口径）。
            serde_json::from_value(serde_json::to_value(&s).expect("card config serializes"))
                .expect("card config round-trips between mirror shapes")
        }
        _ => cfg.card.clone(),
    }
}

fn shellexpand_dir(path: &str) -> PathBuf {
    let expanded = if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    };
    expanded
}
