//! Model 列表抓取（add-fetch-models D1/D4）：自旧 router admin probe 面
//! （`/admin/providers/{name}/probe` 的 `fetch_models`，已被
//! make-core-own-provider-data 下线）抽取的可复用实现。core 的 providers 域
//! 抓取 op 直调本模块，不重复实现协议形状。
//!
//! 契约（specs delta 原文口径）：
//! - URL 候选顺序：`{base_url_openai_chat}/models` →
//!   `{base_url_openai_responses}/models` → `{base_url_anthropic}/v1/models`。
//!   **只按槽位有无选一个 URL**——失败不回退其他槽位（provider-management
//!   spec「single-URL probe choice」scenario；design D4 明确否决跨候选重试）。
//! - 一次只读 GET，5s 总超时，无重试（D4：one bounded, sanitized call）。
//! - 解析只认 `data[].id`（OpenAI 与 Anthropic envelope 同形状）。
//! - 错误串脱敏：只含状态码/类别，绝不含 key 与上游 body 内容。

use serde_json::Value;
use std::time::Duration;

/// 抓取目标 base url 的协议族（决定 `/models` 的追加形状）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchBaseKind {
    /// OpenAI 家族：`{base}/models`。
    OpenAi,
    /// Anthropic：`{base}/v1/models`。
    Anthropic,
}

/// 单次抓取的超时上限（provider-management spec「5 s timeout」）。
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// 抓取专用客户端。进程内复用连接池；5s 上限由 [`fetch_models`] 在调用层
/// 用 `tokio::time::timeout` 兜底执行（不依赖 builder 成功与否）。
pub fn fetch_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// 去掉末尾 `/`（反复 normalize 到单一 url 拼接；与旧卡片 helper 同规则）。
fn trim_trailing_slash(s: &str) -> &str {
    let mut end = s.len();
    while end > 0 && s.as_bytes()[end - 1] == b'/' {
        end -= 1;
    }
    &s[..end]
}

/// 槽位优先级解析：openai_chat → openai_responses → anthropic。三槽全空 →
/// `None`（调用方回 typed rejection naming that reason，且不发上游请求）。
pub fn resolve_fetch_url(
    base_url_openai_chat: Option<&str>,
    base_url_openai_responses: Option<&str>,
    base_url_anthropic: Option<&str>,
) -> Option<(String, FetchBaseKind)> {
    fn nonempty(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|s| !s.is_empty())
    }
    if let Some(base) = nonempty(base_url_openai_chat) {
        return Some((format!("{}/models", trim_trailing_slash(base)), FetchBaseKind::OpenAi));
    }
    if let Some(base) = nonempty(base_url_openai_responses) {
        return Some((format!("{}/models", trim_trailing_slash(base)), FetchBaseKind::OpenAi));
    }
    if let Some(base) = nonempty(base_url_anthropic) {
        return Some((
            format!("{}/v1/models", trim_trailing_slash(base)),
            FetchBaseKind::Anthropic,
        ));
    }
    None
}

/// GET 上游 model 列表（OpenAI `data[].id` / Anthropic `data[].id` 两种形状）。
/// 错误串脱敏：只含状态码/类别，不含 key 与 body。解析不出 `data` 数组 →
/// 空 Vec（HTTP 200 下的真实空列表；调用方决定呈现口径）。
pub async fn fetch_models(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
) -> Result<Vec<String>, String> {
    // D4 的硬上界在调用层执行：builder 失败退化出的默认客户端也不会越过 5s。
    let send = async {
        let mut req = client.get(url);
        if let Some(k) = key {
            req = req.bearer_auth(k);
        }
        req.send().await
    };
    let resp = match tokio::time::timeout(FETCH_TIMEOUT, send).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            // reqwest 错误文本可能带 URL（不含 key——key 走 Authorization 头），
            // 但也可能带代理/IO 细节；这里只保留类别 + 状态码（若有）。
            let cat = if e.is_timeout() {
                "超时"
            } else if e.is_connect() {
                "连接失败"
            } else {
                "请求失败"
            };
            return Err(format!(
                "{cat}: {}",
                e.status().map(|s| s.to_string()).unwrap_or_default()
            ));
        }
        Err(_elapsed) => return Err("超时: 5s".into()),
    };
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body: Value = resp
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| "响应不是 JSON".to_string())?;
    let list = body
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 槽位优先级：chat > responses > anthropic；全空 → None。
    #[test]
    fn resolve_fetch_url_follows_slot_priority() {
        let (url, kind) = resolve_fetch_url(
            Some("https://chat.example/v1"),
            Some("https://resp.example/v1"),
            Some("https://anth.example"),
        )
        .unwrap();
        assert_eq!(url, "https://chat.example/v1/models");
        assert_eq!(kind, FetchBaseKind::OpenAi);

        let (url, kind) = resolve_fetch_url(None, Some("https://resp.example/v1"), Some("https://anth.example"))
            .unwrap();
        assert_eq!(url, "https://resp.example/v1/models");
        assert_eq!(kind, FetchBaseKind::OpenAi);

        let (url, kind) = resolve_fetch_url(None, None, Some("https://anth.example/")).unwrap();
        assert_eq!(url, "https://anth.example/v1/models");
        assert_eq!(kind, FetchBaseKind::Anthropic);

        assert!(resolve_fetch_url(None, None, None).is_none());
        // 空白串视同未配置。
        assert!(resolve_fetch_url(Some("  "), Some(""), None).is_none());
    }

    /// 尾斜杠 normalize：多根尾斜杠归一，不产生 `//models`。
    #[test]
    fn resolve_fetch_url_trims_trailing_slashes() {
        let (url, _) = resolve_fetch_url(Some("https://api.example.com/v1///"), None, None).unwrap();
        assert_eq!(url, "https://api.example.com/v1/models");
    }

    /// fetch_models：HTTP 200 + `data[].id` → id 列表（本地内存 mock，无外联）。
    #[tokio::test]
    async fn fetch_models_parses_data_ids() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let body = r#"{"object":"list","data":[{"id":"m-one"},{"id":"m-two"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let client = fetch_client();
        let models = fetch_models(&client, &format!("http://{addr}/v1/models"), Some("sk-x"))
            .await
            .expect("200 with data[] parses");
        assert_eq!(models, vec!["m-one".to_string(), "m-two".to_string()]);
    }

    /// fetch_models：非 2xx → 只含状态码的错误（不带 body、不带 key）。
    #[tokio::test]
    async fn fetch_models_sanitizes_error_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let body = r#"{"error":"secret-upstream-detail"}"#;
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let client = fetch_client();
        let err = fetch_models(&client, &format!("http://{addr}/v1/models"), Some("sk-leaky"))
            .await
            .expect_err("401 must be an error");
        assert!(err.contains("401"), "error names the status: {err}");
        assert!(!err.contains("secret-upstream-detail"), "no upstream body: {err}");
        assert!(!err.contains("sk-leaky"), "no key material: {err}");
    }

    /// fetch_models：连接不通 → 类别化错误（不 panic、被 5s 上界兜住）。
    #[tokio::test]
    async fn fetch_models_connection_failure_is_category_error() {
        // 端口 1（tcpmux）在本环境无监听者：连接立即被拒。
        let client = fetch_client();
        let err = fetch_models(&client, "http://127.0.0.1:1/v1/models", None)
            .await
            .expect_err("unroutable port must fail");
        assert!(
            err.contains("连接失败") || err.contains("超时"),
            "category error (refused or bounded timeout): {err}"
        );
    }
}
