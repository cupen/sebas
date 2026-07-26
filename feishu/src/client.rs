use crate::events::SessionKey;
use serde::{Deserialize, Serialize};

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

    /// Fetches tenant_access_token via HTTP (one-shot, not on hot path).
    pub async fn fetch_token(&self, http: &reqwest::Client) -> anyhow::Result<FeishuToken> {
        let url = "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal";
        let body = serde_json::json!({
            "app_id": self.config.app_id,
            "app_secret": self.config.app_secret,
        });
        let resp: TokenResponse = http.post(url).json(&body).send().await?.json().await?;
        if resp.code != 0 {
            anyhow::bail!("feishu auth failed: code={} msg={}", resp.code, resp.msg);
        }
        let expires_at = chrono::Utc::now().timestamp() + resp.expire as i64 - 60; // refresh 60s early
        Ok(FeishuToken {
            access_token: resp.tenant_access_token,
            expires_at,
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: String,
    expire: i64,
}

impl FeishuClient {
    pub async fn send_card(
        &self,
        http: &reqwest::Client,
        token: &str,
        key: &SessionKey,
        card_json: serde_json::Value,
    ) -> anyhow::Result<String> {
        let receive_id = key.chat_id.clone();
        let url = if key.thread_id.is_some() {
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
        } else {
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
        };
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": serde_json::to_string(&card_json)?,
        });
        let resp: ApiResponse<MessageOut> = http
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("send_card failed: {} {}", resp.code, resp.msg);
        }
        Ok(resp.data.message_id.unwrap_or_default())
    }

    pub async fn update_card(
        &self,
        http: &reqwest::Client,
        token: &str,
        message_id: &str,
        card_json: serde_json::Value,
    ) -> anyhow::Result<()> {
        let url = format!("https://open.feishu.cn/open-apis/im/v1/messages/{message_id}");
        let body = serde_json::json!({ "content": serde_json::to_string(&card_json)? });
        let resp: ApiResponse<()> = http
            .patch(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("update_card failed: {} {}", resp.code, resp.msg);
        }
        Ok(())
    }

    pub async fn react(
        &self,
        http: &reqwest::Client,
        token: &str,
        message_id: &str,
        emoji_type: &str,
    ) -> anyhow::Result<()> {
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{message_id}/reactions"
        );
        let body = serde_json::json!({ "reaction_type": { "emoji_type": emoji_type } });
        let resp: ApiResponse<serde_json::Value> = http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("react failed: {} {}", resp.code, resp.msg);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    code: i32,
    msg: String,
    #[serde(default)]
    data: T,
    #[serde(default)]
    #[allow(non_snake_case, dead_code)]
    message_id: Option<String>,
}

#[derive(Default, Deserialize)]
struct MessageOut {
    #[allow(non_snake_case)]
    message_id: Option<String>,
}