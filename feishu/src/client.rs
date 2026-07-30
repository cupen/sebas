use crate::events::SessionKey;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub owner_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuToken {
    pub access_token: String,
    pub expires_at: i64, // unix seconds
}

/// Placeholder struct — actual WS connection is built in Task 8.
#[derive(Clone)]
pub struct FeishuClient {
    pub config: FeishuConfig,
}

impl FeishuClient {
    pub fn new(config: FeishuConfig) -> Self {
        Self { config }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: String,
    expire: i64,
}

/// Holds the tenant_access_token and refreshes it on demand.
/// `state` starts empty/expired so the first `token()` call fetches.
#[derive(Clone)]
pub struct TokenManager {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    token_url: String,
    state: std::sync::Arc<Mutex<FeishuToken>>,
}

impl TokenManager {
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self::with_url(
            app_id,
            app_secret,
            "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal".into(),
        )
    }

    pub fn with_url(app_id: String, app_secret: String, token_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            app_id,
            app_secret,
            token_url,
            state: std::sync::Arc::new(Mutex::new(FeishuToken {
                access_token: String::new(),
                expires_at: 0,
            })),
        }
    }

    /// Test hook (SEBAS_TEST_FAKE_TOKEN): preset a valid token; no HTTP is
    /// needed until it expires an hour later.
    pub fn with_stub_token(access_token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            app_id: String::new(),
            app_secret: String::new(),
            token_url: String::new(),
            state: std::sync::Arc::new(Mutex::new(FeishuToken {
                access_token: access_token.into(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
            })),
        }
    }

    pub async fn token(&self) -> anyhow::Result<String> {
        let mut g = self.state.lock().await;
        if chrono::Utc::now().timestamp() >= g.expires_at {
            *g = self.fetch().await?;
        }
        Ok(g.access_token.clone())
    }

    pub async fn force_refresh(&self) -> anyhow::Result<()> {
        let fresh = self.fetch().await?;
        *self.state.lock().await = fresh;
        Ok(())
    }

    async fn fetch(&self) -> anyhow::Result<FeishuToken> {
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });
        let resp: TokenResponse = self
            .http
            .post(&self.token_url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("feishu auth failed: code={} msg={}", resp.code, resp.msg);
        }
        Ok(FeishuToken {
            access_token: resp.tenant_access_token,
            expires_at: chrono::Utc::now().timestamp() + resp.expire as i64 - 60,
        })
    }
}

impl FeishuClient {
    /// POST a card-related JSON body with the shared retry policy:
    /// any business `code != 0` forces a token refresh and exactly one retry.
    pub async fn post_card_with_retry(
        &self,
        http: &reqwest::Client,
        tokens: &TokenManager,
        url: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<String> {
        #[derive(serde::Deserialize)]
        struct R {
            code: i32,
            msg: String,
            #[serde(default)]
            data: MessageOut,
        }
        let mut attempt = 0;
        loop {
            let token = tokens.token().await?;
            let resp: R = http
                .post(url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?
                .json()
                .await?;
            if resp.code == 0 {
                return Ok(resp.data.message_id.unwrap_or_default());
            }
            attempt += 1;
            if attempt > 1 {
                anyhow::bail!("feishu api failed after token refresh: {} {}", resp.code, resp.msg);
            }
            tokens.force_refresh().await?;
        }
    }

    /// Same retry policy for endpoints that return no payload. Takes the HTTP
    /// method so `update_card` (PATCH) and `react` (POST) share one path.
    pub async fn request_with_retry(
        &self,
        http: &reqwest::Client,
        tokens: &TokenManager,
        method: reqwest::Method,
        url: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<()> {
        #[derive(serde::Deserialize)]
        struct R {
            code: i32,
            msg: String,
        }
        let mut attempt = 0;
        loop {
            let token = tokens.token().await?;
            let resp: R = http
                .request(method.clone(), url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?
                .json()
                .await?;
            if resp.code == 0 {
                return Ok(());
            }
            attempt += 1;
            if attempt > 1 {
                anyhow::bail!("feishu api failed after token refresh: {} {}", resp.code, resp.msg);
            }
            tokens.force_refresh().await?;
        }
    }

    pub async fn send_card(
        &self,
        http: &reqwest::Client,
        tokens: &TokenManager,
        key: &SessionKey,
        card_json: serde_json::Value,
    ) -> anyhow::Result<String> {
        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id";
        let body = serde_json::json!({
            "receive_id": key.chat_id,
            "msg_type": "interactive",
            "content": serde_json::to_string(&card_json)?,
        });
        self.post_card_with_retry(http, tokens, url, body).await
    }

    pub async fn update_card(
        &self,
        http: &reqwest::Client,
        tokens: &TokenManager,
        message_id: &str,
        card_json: serde_json::Value,
    ) -> anyhow::Result<()> {
        let url = format!("https://open.feishu.cn/open-apis/im/v1/messages/{message_id}");
        let body = serde_json::json!({ "content": serde_json::to_string(&card_json)? });
        self.request_with_retry(http, tokens, reqwest::Method::PATCH, &url, body)
            .await
    }

    pub async fn react(
        &self,
        http: &reqwest::Client,
        tokens: &TokenManager,
        message_id: &str,
        emoji_type: &str,
    ) -> anyhow::Result<()> {
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{message_id}/reactions"
        );
        let body = serde_json::json!({ "reaction_type": { "emoji_type": emoji_type } });
        self.request_with_retry(http, tokens, reqwest::Method::POST, &url, body)
            .await
    }
}

#[derive(Default, Deserialize)]
struct MessageOut {
    #[allow(non_snake_case)]
    message_id: Option<String>,
}
