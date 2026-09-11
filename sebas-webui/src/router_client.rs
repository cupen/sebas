//! webui → router admin API 客户端（make-core-own-provider-data 3.2 后收缩）。
//!
//! provider / alias / defaults / preset 的管理面已改由 core 状态库承载
//! （`routes.rs` 走 `SessionBackend` 的 state seam），本客户端只剩一个合法
//! 用途：代理 router 的 `POST /admin/reload`（读侧配置刷新，不写 provider
//! 数据）。其余 admin 端点在 router 侧已下线（404），不再保留会打空枪的
//! 客户端方法。
//!
//! base = `http://<router.listen>`（来自启动快照 `RouterInfo.listen`）；
//! Bearer `SEBAS_CONTROL_SECRET`（env）；3s 超时（页面渲染不能被挂死的
//! router 拖死——超时/连接失败由调用方走降级路径）。

use serde_json::Value;

#[derive(Clone)]
pub struct RouterClient {
    base: String,
    secret: String,
    client: reqwest::Client,
}

/// router 不可达/超时/非 2xx 的统一错误面。message 脱敏（不含 secret）。
#[derive(Debug)]
pub struct RouterClientError {
    /// 透传给前端的 HTTP 状态：4xx 原样（400 校验 / 404 / 409 重名），
    /// router 不可达/读失败等本地问题 → 502。
    pub status: axum::http::StatusCode,
    pub message: String,
}

impl RouterClientError {
    fn unreachable(message: String) -> Self {
        Self { status: axum::http::StatusCode::BAD_GATEWAY, message }
    }
    fn from_status(status: reqwest::StatusCode, path: &str, message: Option<String>) -> Self {
        let status = axum::http::StatusCode::from_u16(status.as_u16())
            .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
        Self {
            status,
            message: message.unwrap_or_else(|| format!("router admin {path} 返回 {status}")),
        }
    }
}

impl std::fmt::Display for RouterClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RouterClientError {}

impl RouterClient {
    /// `listen` 是启动快照里的 router 监听地址（如 `127.0.0.1:7897`）。
    pub fn new(listen: &str) -> Self {
        let secret = std::env::var("SEBAS_CONTROL_SECRET").unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        RouterClient {
            base: format!("http://{listen}"),
            secret,
            client,
        }
    }

    /// 无 secret（admin 面拒绝非 loopback bearer 请求）→ 调用方 503。
    pub fn has_secret(&self) -> bool {
        !self.secret.is_empty()
    }

    pub async fn reload(&self) -> Result<Value, RouterClientError> {
        self.post_json("/admin/reload", None).await
    }

    async fn post_json(&self, path: &str, body: Option<&Value>) -> Result<Value, RouterClientError> {
        let mut req = self.client.post(format!("{}{path}", self.base));
        if !self.secret.is_empty() {
            req = req.bearer_auth(&self.secret);
        }
        if let Some(b) = body {
            req = req
                .header("content-type", "application/json")
                .body(serde_json::to_string(b).unwrap_or_default());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RouterClientError::unreachable(format!("router 不可达: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| RouterClientError::unreachable(format!("读响应失败: {e}")))?;
        // 2xx 才是成功；4xx/5xx 把 router 的 error message + 状态码原样带给前端。
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from));
            return Err(RouterClientError::from_status(status, path, msg));
        }
        serde_json::from_str(&text)
            .map_err(|e| RouterClientError::unreachable(format!("响应非 JSON: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post as rpost;
    use serde_json::json;

    /// mock router admin 面：记录 bearer，回固定 JSON。
    async fn mock_admin() -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/admin/reload",
            rpost(|h: axum::http::HeaderMap| async move {
                // 回显收到的 Authorization（测试断言 bearer 注入）。
                let auth = h
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("none")
                    .to_string();
                axum::Json(json!({"reloaded": true, "auth": auth}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("{addr}"), handle)
    }

    #[tokio::test]
    async fn reload_forwards_with_bearer() {
        let (addr, _h) = mock_admin().await;
        let c = RouterClient::new(&addr);
        // 无 secret 也能 POST（loopback），bearer 缺省。
        let v = c.reload().await.unwrap();
        assert_eq!(v["reloaded"], true);

        // 拒绝面：非 2xx 带状态码回传。
        let dead = RouterClient::new("127.0.0.1:59991");
        let r = dead.reload().await;
        assert!(r.is_err(), "不可达须报错");
    }
}
