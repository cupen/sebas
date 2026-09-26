//! 会话自动标题（add-agent-settings-and-session-titles 6.1）：一次性
//! one-shot LLM 调用的小型模块，模式照抄 `sebas_router::probe`——
//! reqwest 直连 provider、5s 超时、无重试、失败静默。
//!
//! - 目标解析（[`resolve_title_target`]）：URL 槽位 `base_url_anthropic`
//!   优先（Anthropic 协议）、`base_url_openai_chat` 兜底（OpenAI chat
//!   协议）；凭据 = 条目明文 `api_key` → `api_key_env` 环境变量 → 无。
//!   provider 与 model 取 providers 域 `default_selection`（调用方解析，
//!   本模块只吃单条 provider 条目）。
//! - 生成（[`generate_title`]）：`max_tokens` 压小（64）、单轮 prompt
//!   「只返回标题原文」；响应解析只认 Anthropic `content[].text` 与
//!   OpenAI `choices[].message.content`。任何失败（未配 URL / 网络 /
//!   4xx5xx / 超时 / 空答）→ `None`，调用方静默回退首条消息预览。
//! - 清洗（[`sanitize_title`]）：换行折叠为空格、掐头去尾、≤40 codepoints
//!   截断；空串 → `None`。
//!
//! 本模块**不**新增对 sebas-agent / sebas-router 的依赖（决策 6），也不
//! 阻塞回合路径——调用方在 `tokio::spawn` 里跑它。

use serde_json::{json, Value};
use std::time::Duration;

/// 单次标题调用的超时上限（与 probe 的 5s 同一初值）。
pub const TITLE_TIMEOUT: Duration = Duration::from_secs(5);
/// max_tokens 压小（标题用不了多少 token；省钱 + 防跑飞）。
pub const TITLE_MAX_TOKENS: u32 = 64;
/// 标题长度上限（codepoints；design 初值，行为合同只要求「有上限」）。
pub const TITLE_MAX_CODEPOINTS: usize = 40;

/// URL 槽位的协议族（决定请求形状与路径拼接）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleProtocol {
    /// Anthropic：`POST {base}/v1/messages`，`x-api-key` 头。
    Anthropic,
    /// OpenAI chat completions：`POST {base}/chat/completions`，bearer。
    OpenAiChat,
}

/// 一次标题调用的目标：URL、凭据、模型与协议族。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleTarget {
    pub url: String,
    pub protocol: TitleProtocol,
    pub key: Option<String>,
    pub model: String,
}

/// 标题生成专用客户端（5s 上界由 builder + 调用层 `tokio::time::timeout`
/// 双保险）。
pub fn title_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TITLE_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// 去掉末尾 `/`（与 probe 同规则）。
fn trim_trailing_slash(s: &str) -> &str {
    let mut end = s.len();
    while end > 0 && s.as_bytes()[end - 1] == b'/' {
        end -= 1;
    }
    &s[..end]
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// provider 条目 + 默认模型 → 标题调用目标。槽位优先级 anthropic →
/// openai_chat；两者皆未配 → `None`（调用方静默回退，不发请求）。
pub fn resolve_title_target(item: &crate::state_store::Item, model: &str) -> Option<TitleTarget> {
    let model = nonempty(Some(model))?.to_string();
    let url_slots = (
        nonempty(item.get("base_url_anthropic").and_then(Value::as_str)),
        nonempty(item.get("base_url_openai_chat").and_then(Value::as_str)),
    );
    let (url, protocol) = match url_slots.0 {
        Some(base) => (format!("{}/v1/messages", trim_trailing_slash(base)), TitleProtocol::Anthropic),
        None => (
            format!(
                "{}/chat/completions",
                trim_trailing_slash(url_slots.1?)
            ),
            TitleProtocol::OpenAiChat,
        ),
    };
    // 凭据：明文 api_key → api_key_env 环境变量 → None（匿名请求）。
    let plain = nonempty(item.get("api_key").and_then(Value::as_str)).map(str::to_string);
    let key = plain.or_else(|| {
        let env_name =
            nonempty(item.get("api_key_env").and_then(Value::as_str))?;
        std::env::var(env_name).ok().filter(|v| !v.trim().is_empty())
    });
    Some(TitleTarget {
        url,
        protocol,
        key,
        model,
    })
}

/// 标题清洗：换行/连续空白折叠为单空格、掐头去尾、按 codepoints 截断到
/// 上限。清洗后为空 → `None`（不写空标题）。
pub fn sanitize_title(raw: &str) -> Option<String> {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let capped: String = collapsed.chars().take(TITLE_MAX_CODEPOINTS).collect();
    let capped = capped.trim().to_string();
    (!capped.is_empty()).then_some(capped)
}

/// 生成标题的统一 prompt（单轮；只返回标题原文）。
pub fn title_prompt(first_message: &str) -> String {
    let message: String = first_message.chars().take(2000).collect();
    format!(
        "为以下用户消息生成一个简短标题（不超过 20 个字）。\
只返回标题原文：不要引号、不要前缀（如「标题：」）、不要任何解释。\n\n用户消息：{message}"
    )
}

/// 一次性标题调用。任何失败 → `None`（错误串只进 debug 日志，不上界面
/// ——spec「fall back silently … no error is surfaced to the operator」）。
pub async fn generate_title(
    client: &reqwest::Client,
    target: &TitleTarget,
    first_message: &str,
) -> Option<String> {
    let body = match target.protocol {
        TitleProtocol::Anthropic => json!({
            "model": target.model,
            "max_tokens": TITLE_MAX_TOKENS,
            "messages": [{"role": "user", "content": title_prompt(first_message)}],
        }),
        TitleProtocol::OpenAiChat => json!({
            "model": target.model,
            "max_tokens": TITLE_MAX_TOKENS,
            "messages": [{"role": "user", "content": title_prompt(first_message)}],
        }),
    };
    let mut req = client.post(&target.url).json(&body);
    req = match target.protocol {
        TitleProtocol::Anthropic => req
            .header("x-api-key", target.key.as_deref().unwrap_or(""))
            .header("anthropic-version", "2023-06-01"),
        TitleProtocol::OpenAiChat => match target.key.as_deref() {
            Some(k) => req.bearer_auth(k),
            None => req,
        },
    };
    let resp = match tokio::time::timeout(TITLE_TIMEOUT, req.send()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "auto-title: request failed");
            return None;
        }
        Err(_) => {
            tracing::debug!("auto-title: timed out");
            return None;
        }
    };
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), "auto-title: upstream error");
        return None;
    }
    let parsed = resp.json::<Value>().await.ok()?;
    let text = match target.protocol {
        TitleProtocol::Anthropic => parsed
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .next()
            }),
        TitleProtocol::OpenAiChat => parsed
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str),
    }?;
    sanitize_title(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// 槽位与凭据优先级：anthropic > openai_chat；明文 key > env；两者皆无
    /// URL → None。
    #[test]
    fn resolve_title_target_follows_slot_priority() {
        let mut item = crate::state_store::Item::new();
        assert!(resolve_title_target(&item, "m").is_none(), "无槽位 → None");

        item.insert("base_url_openai_chat".into(), Value::String("https://chat.example/v1".into()));
        let t = resolve_title_target(&item, "deepseek-chat").unwrap();
        assert_eq!(t.url, "https://chat.example/v1/chat/completions");
        assert_eq!(t.protocol, TitleProtocol::OpenAiChat);

        item.insert("base_url_anthropic".into(), Value::String("https://anth.example/".into()));
        let t = resolve_title_target(&item, "claude-x").unwrap();
        assert_eq!(t.url, "https://anth.example/v1/messages");
        assert_eq!(t.protocol, TitleProtocol::Anthropic);

        item.insert("api_key".into(), Value::String("sk-plain".into()));
        let t = resolve_title_target(&item, "m").unwrap();
        assert_eq!(t.key.as_deref(), Some("sk-plain"));
        item.remove("api_key");
        item.insert("api_key_env".into(), Value::String("SEBAS_TITLE_TEST_KEY".into()));
        // env 未设置 → None（匿名）；设置了 → 跟随。
        let t = resolve_title_target(&item, "m").unwrap();
        assert_eq!(t.key, None);
    }

    /// 空模型名 = 未配置默认模型 → None（spec「no default provider … falls
    /// back silently」的模型半边）。
    #[test]
    fn blank_model_yields_no_target() {
        let mut item = crate::state_store::Item::new();
        item.insert("base_url_anthropic".into(), Value::String("https://x".into()));
        assert!(resolve_title_target(&item, "").is_none());
        assert!(resolve_title_target(&item, "   ").is_none());
    }

    /// 清洗：换行折叠单行、掐头去尾、40 codepoints 截断、空串 → None。
    #[test]
    fn sanitize_collapses_and_caps() {
        assert_eq!(
            sanitize_title("  第一行\n第二行\t第三行  ").as_deref(),
            Some("第一行 第二行 第三行")
        );
        let long = "啊".repeat(50);
        let capped = sanitize_title(&long).unwrap();
        assert_eq!(capped.chars().count(), 40);
        assert_eq!(sanitize_title("   \n  "), None);
    }

    /// 本地内存 mock 上游：Anthropic 形状（content[].text）解析出标题。
    #[tokio::test]
    async fn generate_title_parses_anthropic_shape() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"content":[{"type":"text","text":"  会话\n标题  "}],"usage":{}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let target = TitleTarget {
            url: format!("http://{addr}/v1/messages"),
            protocol: TitleProtocol::Anthropic,
            key: Some("sk-x".into()),
            model: "test-model".into(),
        };
        let title = generate_title(&title_client(), &target, "帮我把这段话翻译成英文，谢谢").await;
        assert_eq!(title.as_deref(), Some("会话 标题"), "解析 + 清洗一步到位");
    }

    /// OpenAI 形状（choices[0].message.content）解析出标题。
    #[tokio::test]
    async fn generate_title_parses_openai_shape() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"choices":[{"message":{"role":"assistant","content":"翻译请求"}}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let target = TitleTarget {
            url: format!("http://{addr}/chat/completions"),
            protocol: TitleProtocol::OpenAiChat,
            key: None,
            model: "test-model".into(),
        };
        let title = generate_title(&title_client(), &target, "hello").await;
        assert_eq!(title.as_deref(), Some("翻译请求"));
    }

    /// HTTP 失败 / 连接失败 / 畸形响应 → None（静默，不 panic、不外泄错误）。
    #[tokio::test]
    async fn generate_title_fails_silently() {
        // 上游 5xx。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let target = TitleTarget {
            url: format!("http://{addr}/v1/messages"),
            protocol: TitleProtocol::Anthropic,
            key: None,
            model: "m".into(),
        };
        assert!(generate_title(&title_client(), &target, "x").await.is_none());

        // 连接被拒（端口 1 无监听者）。
        let target = TitleTarget {
            url: "http://127.0.0.1:1/v1/messages".into(),
            protocol: TitleProtocol::Anthropic,
            key: None,
            model: "m".into(),
        };
        assert!(generate_title(&title_client(), &target, "x").await.is_none());

        // 200 但形状不对 → None。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr2 = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = r#"{"unexpected":true}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let target = TitleTarget {
            url: format!("http://{addr2}/v1/messages"),
            protocol: TitleProtocol::Anthropic,
            key: None,
            model: "m".into(),
        };
        assert!(generate_title(&title_client(), &target, "x").await.is_none());
    }
}
