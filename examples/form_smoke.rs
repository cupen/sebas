//! CRUD 表单冒烟测试：用真实飞书客户端走一遍 列表 → 新增 → 填写提交 →
//! 编辑 → 删除，验证 card JSON 2.0 `form` 容器的渲染、`form_value` 回调
//! 与卡片原地更新。
//!
//! 用法（凭证从 config.toml 或 env 读取，不要把密钥贴进聊天记录）：
//!
//! ```text
//! # config.toml 里已有 [feishu] app_id / app_secret / owner_id 时：
//! cargo run -p sebas --example form_smoke -- --config config/config.toml
//!
//! # 或者指定目标会话 / 用户：
//! cargo run -p sebas --example form_smoke -- --chat-id oc_xxx
//! cargo run -p sebas --example form_smoke -- --open-id ou_xxx
//! ```
//!
//! 前置条件：应用的卡片回传交互（card.action.trigger）已通过长连接订阅
//! （跑 sebas 的应用通常已具备）；飞书客户端 V6.6+（form 容器要求）。

use feishu::client::{FeishuClient, TokenManager};
use feishu::events::{FeishuEnvelope, FeishuIn, SessionKey};
use feishu::messages::{ReceiveIdType, SendCardRequest};
use open_lark::Config as LarkConfig;
use open_lark::ws_client::{EventDispatcherHandler, EventHandler, LarkWsClient, WsClientError};
use router::Out;
use router::crud::{CrudForm, FileStore, ProviderForms};
use sebas::config::Config;
use sebas::provider::build_form;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// 初始卡片的发送目标：单聊（chat_id）或用户 DM（open_id）。
#[derive(Debug, Clone)]
enum Target {
    Chat(String),
    Open(String),
}

impl Target {
    /// SessionKey 只用于 CRUD 内部的状态流转；open_id 模式下真实会话
    /// 由回调的 context.open_chat_id 决定，这里先占位空串。
    fn placeholder_key(&self) -> SessionKey {
        match self {
            Target::Chat(chat_id) => SessionKey {
                chat_id: chat_id.clone(),
                thread_id: None,
            },
            Target::Open(_) => SessionKey {
                chat_id: String::new(),
                thread_id: None,
            },
        }
    }
}

#[derive(Clone)]
struct Handler {
    http: reqwest::Client,
    client: FeishuClient,
    tokens: TokenManager,
    forms: Arc<ProviderForms>,
    /// 发送目标：启动时可由 CLI 指定；否则等第一条入站消息自动确定。
    target: Arc<tokio::sync::Mutex<Option<Target>>>,
}

impl EventHandler for Handler {
    fn handle(
        &self,
        payload: &[u8],
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let this = self.clone();
        let payload = payload.to_vec();
        tokio::spawn(async move {
            if let Err(e) = this.process(&payload).await {
                warn!(error = %e, "form_smoke: failed to process inbound frame");
            }
        });
        Ok(())
    }
}

impl Handler {
    async fn process(&self, payload: &[u8]) -> anyhow::Result<()> {
        let text = std::str::from_utf8(payload)?;
        let env: FeishuEnvelope = serde_json::from_str(text)?;
        let Some(in_ev) = env.into_event("") else {
            return Ok(());
        };
        match in_ev {
            FeishuIn::Text { key, text, .. } => {
                self.bootstrap(key, &text).await?;
            }
            FeishuIn::FormCb {
                key,
                value,
                form_value,
                message_id,
                ..
            } => {
                info!(?key, ?form_value, ?message_id, "form submission received");
                let form_name = value.get("form").and_then(Value::as_str).unwrap_or("");
                let Some(form) = self.forms.dispatch(form_name) else {
                    warn!(form_name, "unwired form callback ignored");
                    return Ok(());
                };
                let out = form.handle(key, &value, &form_value, message_id).await;
                self.dispatch(out).await?;
            }
            FeishuIn::ButtonCb { key, action, .. } => {
                // 列表卡上的 新增/编辑/删除 是普通按钮回调（无 form_value），
                // 负载在 action.value 里，message_id 在 context 里。
                let payload = action.value.pointer("/action/value").cloned();
                let op = payload
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(|p| p.get("op"))
                    .and_then(Value::as_str);
                let target = match op {
                    Some("create") => payload
                        .as_ref()
                        .and_then(Value::as_object)
                        .and_then(|p| p.get("form"))
                        .and_then(Value::as_str)
                        .and_then(|n| self.forms.dispatch(n))
                        .map(Arc::clone),
                    Some("edit") | Some("delete") => {
                        let id = payload
                            .as_ref()
                            .and_then(Value::as_object)
                            .and_then(|p| p.get("id"))
                            .and_then(Value::as_str);
                        match id {
                            Some(id) => self.forms.pick_for_edit(id).await,
                            None => None,
                        }
                    }
                    _ => None,
                };
                if let Some(form) = target {
                    let message_id = action
                        .value
                        .pointer("/context/open_message_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    info!(?key, ?payload, ?message_id, "button callback received");
                    let out = form
                        .handle(
                            key,
                            &payload.unwrap_or(Value::Null),
                            &BTreeMap::new(),
                            message_id,
                        )
                        .await;
                    self.dispatch(out).await?;
                }
            }
            other => debug!("ignoring inbound event: {other:?}"),
        }
        Ok(())
    }

    /// 未指定目标时，用第一条入站消息的会话作为发送目标，并发出初始列表卡。
    async fn bootstrap(&self, key: SessionKey, text: &str) -> anyhow::Result<()> {
        let mut target = self.target.lock().await;
        if target.is_some() {
            return Ok(());
        }
        *target = Some(Target::Chat(key.chat_id.clone()));
        drop(target);
        info!(?key, text = %text, "target resolved from inbound message; sending CRUD card");
        let out = self.forms.open(key).await;
        self.dispatch(out).await?;
        Ok(())
    }

    async fn dispatch(&self, out: Out) -> anyhow::Result<()> {
        match out {
            Out::SendCard { card, .. } => {
                let target = self
                    .target
                    .lock()
                    .await
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("发送目标尚未确定"))?;
                match target {
                    Target::Chat(chat_id) => {
                        let key = SessionKey {
                            chat_id,
                            thread_id: None,
                        };
                        self.client
                            .send_card(&self.http, &self.tokens, &key, card, None)
                            .await?;
                    }
                    Target::Open(open_id) => {
                        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
                        let req = SendCardRequest::new(open_id, ReceiveIdType::OpenId, &card);
                        let body = serde_json::to_value(&req)?;
                        self.client
                            .post_card_with_retry(&self.http, &self.tokens, url, body)
                            .await?;
                    }
                }
            }
            Out::UpdateCardByMsgId { msg_id, card, .. } => {
                self.client
                    .update_card(&self.http, &self.tokens, &msg_id, card)
                    .await?;
                info!(%msg_id, "card updated in place");
            }
            other => warn!("unexpected Out for form smoke: {other:?}"),
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 与 src/run.rs 一致：reqwest 0.12 混合依赖图里显式固定 rustls provider，
    // 避免运行时 panic 找不到 CryptoProvider。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut config_path = std::path::PathBuf::from("config/config.toml");
    let mut chat_id: Option<String> = None;
    let mut open_id: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--chat-id" => chat_id = args.next(),
            "--open-id" => open_id = args.next(),
            "--config" => {
                if let Some(p) = args.next() {
                    config_path = p.into();
                }
            }
            "-h" | "--help" => {
                println!(
                    "用法: form_smoke [--config <path>] [--chat-id <oc_xxx> | --open-id <ou_xxx>]\n\
                     凭证从 config.toml 的 [feishu] 段或 SEBAS_FEISHU_APP_ID/SECRET 环境变量读取。"
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let cfg_text = match std::fs::read_to_string(&config_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "config {} 不存在，仅使用 SEBAS_FEISHU_APP_ID / SEBAS_FEISHU_APP_SECRET 环境变量",
                config_path.display()
            );
            String::new()
        }
        Err(e) => anyhow::bail!("读取配置 {} 失败: {e}", config_path.display()),
    };
    let cfg = Config::parse(&cfg_text)?;
    let target = match (chat_id, open_id) {
        (Some(id), _) => Some(Target::Chat(id)),
        (None, Some(id)) => Some(Target::Open(id)),
        (None, None) if !cfg.feishu.owner_id.is_empty() => {
            Some(Target::Open(cfg.feishu.owner_id.clone()))
        }
        _ => None,
    };
    match &target {
        Some(t) => info!(?t, "sending initial CRUD card"),
        None => info!(
            "未指定 --chat-id/--open-id 且 config 无 owner_id：等待机器人收到任意消息后自动确定会话"
        ),
    }

    let http = reqwest::Client::new();
    let client = FeishuClient::new(feishu::client::FeishuConfig {
        app_id: cfg.feishu.app_id.clone(),
        app_secret: cfg.feishu.app_secret.clone(),
        owner_id: cfg.feishu.owner_id.clone(),
    });
    let tokens = TokenManager::new(cfg.feishu.app_id.clone(), cfg.feishu.app_secret.clone());

    // 直接复用 /provider 的真实表单：FileStore overlay 持久化 + preset
    // 规范化，种子来自 config.toml 顶层 [provider.*]。
    let forms = match build_form(&cfg_text) {
        Some(forms) => forms,
        None => anyhow::bail!("provider 表单不可用（overlay 加载失败），见上面的日志"),
    };
    let handler = Handler {
        http,
        client,
        tokens,
        forms: forms.clone(),
        target: Arc::new(tokio::sync::Mutex::new(target.clone())),
    };

    // 先发初始列表卡，然后进入长连接等待交互。
    if let Some(target) = &target {
        handler
            .dispatch(forms.open(target.placeholder_key()).await)
            .await?;
        info!("初始列表卡已发送：请点「＋ 新增（预设/自定义）」→ 填写 → 提交 → 编辑 → 删除");
    } else {
        info!("等待入站消息……");
    }

    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);
    loop {
        let dispatcher = match EventDispatcherHandler::builder()
            .register_raw("im.message.receive_v1", handler.clone())
            .and_then(|b| b.register_raw("card.action.trigger", handler.clone()))
        {
            Ok(d) => d,
            Err(e) => anyhow::bail!("register_raw failed: {e}"),
        };
        let ws_config = Arc::new(
            LarkConfig::builder()
                .app_id(cfg.feishu.app_id.clone())
                .app_secret(cfg.feishu.app_secret.clone())
                .build(),
        );
        info!("connecting to feishu WS");
        let result = LarkWsClient::open(ws_config, dispatcher).await;
        match result {
            Ok(()) => {
                info!("WS session ended cleanly; reconnecting");
                backoff = Duration::from_secs(1);
            }
            Err(WsClientError::ConnectionClosed { reason }) => {
                warn!(?reason, "WS closed; reconnecting");
                backoff = Duration::from_secs(1);
            }
            Err(e) => {
                warn!(error = %e, "WS failed; backing off");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}
