//! 飞书装配入口（extract-im-service M1.4：自 `src/run.rs` 的 `sebas core`
//! 装配段抽出，core 与未来的 `sebas im` 独立进程共用同一装配）。
//!
//! 职责：token 引导（含 `SEBAS_TEST_FAKE_TOKEN=1` 测试桩）、启动问候/
//! 测试消息、`FeishuClient` + `FeishuAdapter` 实例化、WS 循环 spawn 与
//! 入站 `ChannelEvent` 接收端交付。出站呈现泵不属于装配（M1 阶段仍由
//! core 的 `dispatch_out` 驱动，M3 割接后由 im 的渲染队列接管）。

use sebas_channels::ChannelAdapter;
use sebas_feishu::adapter::{FeishuAdapter, FeishuAdapterConfig};
use sebas_feishu::client::{FeishuClient, TokenManager};
use sebas_feishu::messages::{ReceiveIdType, SendTextRequest};
use std::path::PathBuf;
use tracing::{error, info, warn};

/// 装配所需的飞书配置切片（`[feishu]` + `[card]` + 装配面参数）。
#[derive(Debug, Clone)]
pub struct FeishuBootstrapConfig {
    pub app_id: String,
    pub app_secret: String,
    pub owner_id: String,
    pub allowed_chat_types: Vec<String>,
    pub bot_name: String,
    pub hello_msg: String,
    /// `[card]` 渲染配置：由 adapter 解释（theme/truncation/fold）。
    pub card_config: sebas_feishu::cards::CardConfig,
    /// 入站 WS 快照 dump 目录（`--dump-inbound`）；None 关闭。
    pub dump_dir: Option<PathBuf>,
    /// 入站 `ChannelEvent` 通道容量。
    pub channel_buffer: usize,
}

/// 一次飞书装配的产物：adapter + 出站所需句柄。入站 WS 循环由
/// [`FeishuBootstrap::spawn_inbound`] 单独启动（失败沿历史语义：记日志继续）。
pub struct FeishuBootstrap {
    pub adapter: FeishuAdapter,
    pub tokens: TokenManager,
    pub client: FeishuClient,
    pub http: reqwest::Client,
}

impl FeishuBootstrap {
    /// Spawn adapter 的 WS 循环并返回入站 `ChannelEvent` 接收端，调用方
    /// 消费并交给自己的会话面（core：router.dispatch；im 进程：IM 前端）。
    /// spawn 失败不致命：沿用历史行为记错误日志并返回 `None`（无入站继续跑）。
    pub fn spawn_inbound(
        &self,
        channel_buffer: usize,
    ) -> Option<tokio::sync::mpsc::Receiver<sebas_channels::ChannelEvent>> {
        let (inbound_tx, inbound_rx) =
            tokio::sync::mpsc::channel::<sebas_channels::ChannelEvent>(channel_buffer);
        match self.adapter.spawn(inbound_tx) {
            Ok(()) => {
                info!("feishu adapter registered + WS loop spawned");
                Some(inbound_rx)
            }
            Err(e) => {
                error!(error = %e, "failed to spawn feishu adapter; continuing without inbound");
                None
            }
        }
    }
}

/// 装配飞书通道：token 引导 → 问候/测试消息 → adapter 实例化并 spawn WS。
/// 失败以错误字符串返回（调用方映射到各自的错误类型）。
pub async fn bootstrap(
    cfg: FeishuBootstrapConfig,
    test_msg: Option<String>,
) -> Result<FeishuBootstrap, String> {
    // Test affordance: `SEBAS_TEST_FAKE_TOKEN=1` skips the live Feishu auth
    // HTTP call and substitutes a stub token. Used by integration tests that
    // cannot reach the live Feishu API. Off by default; production callers
    // see no behaviour change.
    let tokens = if std::env::var("SEBAS_TEST_FAKE_TOKEN").as_deref() == Ok("1") {
        info!("SEBAS_TEST_FAKE_TOKEN=1; using stub tenant_access_token");
        TokenManager::with_stub_token("t-stub-test")
    } else {
        let tm = TokenManager::new(cfg.app_id.clone(), cfg.app_secret.clone());
        // Startup auth check stays fatal (openspec/specs/acp-driver/spec.md) — 仅在 feishu 启用时。
        tm.token()
            .await
            .map_err(|e| e.to_string())?;
        tm
    };

    let http = reqwest::Client::new();

    // hello_msg: send to the owner (private DM via open_id) if both are set.
    // If owner_id is empty, do nothing.
    if !cfg.hello_msg.is_empty() && !cfg.owner_id.is_empty() {
        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
        let req = SendTextRequest::new(&cfg.owner_id, ReceiveIdType::OpenId, &cfg.hello_msg);
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
        let bearer = tokens.token().await.unwrap_or_default();
        let resp = http
            .post(url)
            .bearer_auth(&bearer)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("test message send: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        info!(%status, body = %body, "test message send result");
        if !status.is_success() {
            return Err(format!("test message failed: {body}"));
        }
    }

    let feishu = FeishuClient::new(sebas_feishu::client::FeishuConfig {
        app_id: cfg.app_id.clone(),
        app_secret: cfg.app_secret.clone(),
        owner_id: cfg.owner_id.clone(),
    });
    let adapter = FeishuAdapter::new(
        feishu.clone(),
        FeishuAdapterConfig {
            app_id: cfg.app_id.clone(),
            app_secret: cfg.app_secret.clone(),
            owner_id: cfg.owner_id.clone(),
            allowed_chat_types: cfg.allowed_chat_types.clone(),
            bot_name: cfg.bot_name.clone(),
            dump_dir: cfg.dump_dir,
            card_config: cfg.card_config,
        },
    );

    Ok(FeishuBootstrap {
        adapter,
        tokens,
        client: feishu,
        http,
    })
}
