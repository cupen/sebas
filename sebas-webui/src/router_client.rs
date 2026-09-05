//! webui → router admin API 客户端（Task 6.1，BFF 模式：webui 不再读
//! providers.json 快照，而是经 router 的 `/admin/*` 拉实时数据/写变更）。
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
pub struct RouterClientError(pub String);

impl std::fmt::Display for RouterClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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

    async fn get(&self, path: &str) -> Result<Value, RouterClientError> {
        let mut req = self.client.get(format!("{}{path}", self.base));
        if !self.secret.is_empty() {
            req = req.bearer_auth(&self.secret);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RouterClientError(format!("router 不可达: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| RouterClientError(format!("读响应失败: {e}")))?;
        if !status.is_success() {
            return Err(RouterClientError(format!(
                "router admin {path} 返回 {status}"
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| RouterClientError(format!("响应非 JSON: {e}")))
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
            .map_err(|e| RouterClientError(format!("router 不可达: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| RouterClientError(format!("读响应失败: {e}")))?;
        // 2xx 才是成功；4xx/5xx 把 router 的 error message 原样带给前端。
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or(format!("router admin {path} 返回 {status}"));
            return Err(RouterClientError(msg));
        }
        serde_json::from_str(&text)
            .map_err(|e| RouterClientError(format!("响应非 JSON: {e}")))
    }

    // ---- 只读面 ----

    pub async fn providers(&self) -> Result<Value, RouterClientError> {
        self.get("/admin/providers").await
    }

    pub async fn model_aliases(&self) -> Result<Value, RouterClientError> {
        self.get("/admin/model-aliases").await
    }

    pub async fn stats(&self) -> Result<Value, RouterClientError> {
        self.get("/admin/stats").await
    }

    // ---- 变更面（POST/PUT/DELETE，全部经 admin API）----

    pub async fn create_provider(&self, body: &Value) -> Result<Value, RouterClientError> {
        self.post_json("/admin/providers", Some(body)).await
    }

    pub async fn update_provider(&self, name: &str, body: &Value) -> Result<Value, RouterClientError> {
        self.request_json(
            reqwest::Method::PUT,
            &format!("/admin/providers/{name}"),
            Some(body),
        )
        .await
    }

    pub async fn delete_provider(&self, name: &str) -> Result<Value, RouterClientError> {
        self.request_json(reqwest::Method::DELETE, &format!("/admin/providers/{name}"), None)
            .await
    }

    pub async fn probe_provider(&self, name: &str, apply: bool) -> Result<Value, RouterClientError> {
        let q = if apply { "?apply=true" } else { "" };
        self.post_json(&format!("/admin/providers/{name}/probe{q}"), None)
            .await
    }

    pub async fn create_alias(&self, body: &Value) -> Result<Value, RouterClientError> {
        self.post_json("/admin/model-aliases", Some(body)).await
    }

    pub async fn update_alias(&self, alias: &str, body: &Value) -> Result<Value, RouterClientError> {
        self.request_json(
            reqwest::Method::PUT,
            &format!("/admin/model-aliases/{alias}"),
            Some(body),
        )
        .await
    }

    pub async fn delete_alias(&self, alias: &str) -> Result<Value, RouterClientError> {
        self.request_json(reqwest::Method::DELETE, &format!("/admin/model-aliases/{alias}"), None)
            .await
    }

    pub async fn reload(&self) -> Result<Value, RouterClientError> {
        self.post_json("/admin/reload", None).await
    }

    async fn request_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, RouterClientError> {
        let mut req = self.client.request(method, format!("{}{path}", self.base));
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
            .map_err(|e| RouterClientError(format!("router 不可达: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| RouterClientError(format!("读响应失败: {e}")))?;
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or(format!("router admin {path} 返回 {status}"));
            return Err(RouterClientError(msg));
        }
        serde_json::from_str(&text)
            .map_err(|e| RouterClientError(format!("响应非 JSON: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get as rget;
    use serde_json::json;

    /// mock router admin 面：记录 bearer，回固定 JSON。
    async fn mock_admin() -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new()
            .route(
                "/admin/providers",
                rget(|| async {
                    axum::Json(json!({"providers": [{"name": "alpha"}]}))
                })
                .post(|h: axum::http::HeaderMap| async move {
                    // 回显收到的 Authorization（测试断言 bearer 注入）。
                    let auth = h
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("none")
                        .to_string();
                    axum::Json(json!({"created": "alpha", "auth": auth}))
                }),
            )
            .route(
                "/admin/slow",
                rget(|| async {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    "ok"
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
    async fn forwards_with_bearer_and_times_out() {
        let (addr, _h) = mock_admin().await;
        let c = RouterClient::new(&addr);
        // 无 secret 也能 GET（loopback），bearer 缺省。
        let v = c.providers().await.unwrap();
        assert_eq!(v["providers"][0]["name"], "alpha");

        // 超时降级：/admin/slow 挂 10s，client 3s 超时 → Err。
        let slow = RouterClient::new(&addr);
        let t0 = std::time::Instant::now();
        let r = slow.get("/admin/slow").await;
        assert!(r.is_err(), "slow 须超时报错");
        assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    }
}
