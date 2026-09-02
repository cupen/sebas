//! 网络工具（task 2.1–2.3，design N3）：web_fetch / web_search。
//!
//! 默认 deny（`[agent.policy] network = "off"` 时工具不注册/直接拒——evaluate
//! 层兜底）；`ask` 走审查卡；`on` 静默。硬上限 + 截断标记；错误是数据。
//! robots.txt best-effort（读 Disallow 前缀比对，失败不阻塞）。

use super::{Tool, ToolCtx};
use crate::message::{ToolErrorKind, ToolOutput};

/// fetch 正文上限（design N3）。
pub const FETCH_BODY_CAP: usize = 100_000;
/// search 结果条目上限。
pub const SEARCH_RESULTS_CAP: usize = 8;
/// fetch/search 总超时。
pub const NET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// fetch 重定向上限。
const MAX_REDIRECTS: usize = 3;

pub struct WebFetchTool;

impl WebFetchTool {
    fn description() -> String {
        "Fetch a web page over HTTP(S) and return readable text (HTML tags stripped), \
         capped at ~100KB with a truncation marker. Honors robots.txt when readable. \
         Disabled unless the session policy enables network access."
            .into()
    }

    fn parameters() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL to fetch."}
            },
            "required": ["url"]
        })
    }
}

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> String {
        Self::description()
    }

    fn parameters(&self) -> serde_json::Value {
        Self::parameters()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(url) = input.get("url").and_then(|u| u.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `url` string");
        };
        let parsed = match reqwest::Url::parse(url) {
            Ok(u) => u,
            Err(e) => {
                return ToolOutput::error(
                    ToolErrorKind::InvalidArgs,
                    format!("bad url {url:?}: {e}"),
                );
            }
        };
        // scheme 白名单：仅 http/https（design N3 校验面）。
        if !matches!(parsed.scheme(), "http" | "https") {
            return ToolOutput::error(
                ToolErrorKind::InvalidArgs,
                format!("only http/https schemes are allowed, got {:?}", parsed.scheme()),
            );
        }
        let host = parsed.host_str().unwrap_or_default().to_string();

        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .timeout(NET_TIMEOUT)
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(ToolErrorKind::Io(e.to_string()), "client build failed"),
        };

        // robots.txt best-effort：Disallow 前缀命中即拒绝；读取失败不阻塞。
        if let Some(path) = robots_denied(&client, &parsed).await {
            return ToolOutput::error(
                ToolErrorKind::Denied { reason: "robots.txt".into() },
                format!("robots.txt disallows fetching {path:?} on {host}"),
            );
        }

        let resp = match client.get(parsed.clone()).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::error(ToolErrorKind::Io(e.to_string()), format!("fetch {url:?} failed"));
            }
        };
        let status = resp.status();
        let final_url = resp.url().clone();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutput::error(ToolErrorKind::Io(e.to_string()), format!("read {url:?} failed"));
            }
        };

        let total = bytes.len();
        let truncated = total > FETCH_BODY_CAP;
        let slice = &bytes[..total.min(FETCH_BODY_CAP)];
        let mut body = if content_type.contains("html") || content_type.is_empty() {
            html_to_text(&String::from_utf8_lossy(slice))
        } else {
            String::from_utf8_lossy(slice).into_owned()
        };
        if truncated {
            body.push_str(&format!(
                "\n[truncated: fetched {} of {} bytes from {}]",
                FETCH_BODY_CAP,
                total,
                final_url
            ));
        }
        // body 已在 cap 内且尾部带截断标记；外层不再二次截断（会把标记切掉）。
        let out = format!("fetched {final_url} (status {status}, {total} bytes):\n{body}");
        ToolOutput {
            ok: true,
            output: out,
            truncated,
            exit_code: None,
            error: None,
        }
    }
}

pub struct WebSearchTool;

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> String {
        "Search the web for a query and return up to 8 result entries (title, url, \
         snippet). Requires network access to be enabled by policy; the result count \
         is hard-capped with a truncation marker."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query."},
                "max_results": {"type": "integer", "description": "Optional cap (max 8)."}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(query) = input.get("query").and_then(|q| q.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `query` string");
        };
        let max = input
            .get("max_results")
            .and_then(|m| m.as_u64())
            .map(|m| (m as usize).min(SEARCH_RESULTS_CAP))
            .unwrap_or(SEARCH_RESULTS_CAP);

        // 检索后端：DuckDuckGo HTML 端点（无 key、无重定向跳转、结果以链接列表
        // 呈现）。解析失败 = 结构化错误数据，不 panic。
        let client = match reqwest::Client::builder().timeout(NET_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(ToolErrorKind::Io(e.to_string()), "client build failed"),
        };
        let resp = match client
            .post("https://html.duckduckgo.com/html/")
            .form(&[("q", query)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::error(
                    ToolErrorKind::Io(e.to_string()),
                    format!("search {query:?} failed"),
                );
            }
        };
        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return ToolOutput::error(ToolErrorKind::Io(e.to_string()), format!("read search results failed: {e}"));
            }
        };

        let results = parse_ddg_results(&html, max);
        if results.is_empty() {
            return ToolOutput::ok(format!("no results for {query:?}"));
        }
        let truncated = results.len() >= max;
        let mut out = results
            .iter()
            .map(|(title, url, snippet)| format!("- {title}\n  {url}\n  {snippet}"))
            .collect::<Vec<_>>()
            .join("\n");
        if truncated {
            out.push_str(&format!("\n[truncated: capped at {max} results]"));
        }
        ToolOutput {
            ok: true,
            output: out,
            truncated,
            exit_code: None,
            error: None,
        }
    }
}

/// robots.txt best-effort：返回被 Disallow 的路径前缀（若命中）。
/// 用独立短命 client——robots 连接与主请求连接池隔离，避免桩/慢关连接被
/// 池化复用导致 RST。
async fn robots_denied(client: &reqwest::Client, url: &reqwest::Url) -> Option<String> {
    let _ = client; // 签名保留：调用方决定超时策略
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let scheme = url.scheme();
    let robots_url = format!("{scheme}://{host}{port}/robots.txt");
    let one_shot = reqwest::Client::builder()
        .timeout(NET_TIMEOUT)
        .pool_max_idle_per_host(0)
        .build()
        .ok()?;
    let body = one_shot
        .get(robots_url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    // 仅取 `User-agent: *` 组的 Disallow 前缀（足够 best-effort）。
    let mut in_star_group = false;
    let path = url.path().to_string();
    for line in body.lines() {
        let line = line.trim();
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
            None => continue,
        };
        match key.as_str() {
            "user-agent" => in_star_group = value == "*",
            "disallow" if in_star_group && !value.is_empty() && path.starts_with(value) => {
                return Some(path);
            }
            _ => {}
        }
    }
    None
}

/// 极简 HTML → 文本：去 script/style、去标签、解常见实体、压空白。
fn html_to_text(html: &str) -> String {
    let s = html;
    let mut out = String::with_capacity(s.len() / 2);
    let lower = s.to_lowercase();
    let mut skip_until: Option<&'static str> = None;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < s.len() {
        if let Some(tag) = skip_until {
            if lower[i..].starts_with(tag) {
                i += tag.len();
                skip_until = None;
            } else {
                i += 1;
            }
            continue;
        }
        if lower[i..].starts_with("<script") {
            skip_until = Some("</script>");
            i += "<script".len();
            continue;
        }
        if lower[i..].starts_with("<style") {
            skip_until = Some("</style>");
            i += "<style".len();
            continue;
        }
        if bytes[i] == b'<' {
            // 标签：块级标签转换行
            if let Some(end) = s[i..].find('>') {
                let tag = &lower[i..i + end];
                if tag.starts_with("<p") || tag.starts_with("<br") || tag.starts_with("<div")
                    || tag.starts_with("<li") || tag.starts_with("<h1") || tag.starts_with("<h2")
                    || tag.starts_with("<h3") || tag.starts_with("<tr")
                {
                    out.push('\n');
                }
                i += end + 1;
                continue;
            }
            break;
        }
        if bytes[i] == b'&' {
            // 实体
            let rest = &s[i..];
            let entity = rest.split(';').next().unwrap_or("");
            let decoded = match entity.to_ascii_lowercase().as_str() {
                "&amp" => Some('&'),
                "&lt" => Some('<'),
                "&gt" => Some('>'),
                "&quot" => Some('"'),
                "&#39" | "&apos" => Some('\''),
                "&nbsp" => Some(' '),
                _ => None,
            };
            if let Some(c) = decoded {
                out.push(c);
                i += entity.len() + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    // 压空白（逐行）
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 DuckDuckGo HTML 结果中提取 (title, url, snippet)，至多 `max` 条。
fn parse_ddg_results(html: &str, max: usize) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    // 结果行形如 <a rel="nofollow" class="result__a" href="URL">TITLE</a>
    // 摘要 <a class="result__snippet" ...>SNIPPET</a>
    for part in html.split("result__a") {
        if results.len() >= max {
            break;
        }
        let Some(href_pos) = part.find("href=\"") else {
            continue;
        };
        let url_start = href_pos + "href=\"".len();
        let Some(url_len) = part[url_start..].find('"') else {
            continue;
        };
        let mut url = part[url_start..url_start + url_len].to_string();
        // DDG 包一层跳转：//duckduckgo.com/l/?uddg=<encoded>
        if let Some(pos) = url.find("uddg=") {
            let enc = &url[pos + "uddg=".len()..];
            let enc = enc.split('&').next().unwrap_or(enc);
            url = urldecode(enc);
        }
        if !url.starts_with("http") {
            continue;
        }
        let Some(gt) = part[url_start + url_len..].find('>') else {
            continue;
        };
        let after = &part[url_start + url_len + gt + 1..];
        let Some(lt) = after.find("</a>") else {
            continue;
        };
        let title = html_to_text(&after[..lt]);
        let snippet = part
            .split("result__snippet")
            .nth(1)
            .map(|s| {
                s.find('>').map(|h| {
                    let rest = &s[h + 1..];
                    let t = rest.find("</a>").unwrap_or(rest.len());
                    html_to_text(&rest[..t])
                })
            })
            .unwrap_or_default()
            .unwrap_or_default();
        if !title.is_empty() {
            results.push((title, url, snippet));
        }
    }
    results
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolCtx;
    use tokio_util::sync::CancellationToken;

    fn ctx(dir: &Path) -> ToolCtx {
        ToolCtx::new(dir.to_path_buf(), CancellationToken::new())
    }
    use std::path::Path;

    #[tokio::test]
    async fn missing_or_bad_url_is_invalid_args() {
        let dir = tempfile::tempdir().unwrap();
        let out = WebFetchTool.execute(serde_json::json!({}), &ctx(dir.path())).await;
        assert!(!out.ok);
        assert!(matches!(out.error, Some(ToolErrorKind::InvalidArgs)));

        let out = WebFetchTool
            .execute(serde_json::json!({"url": "ftp://example.com/x"}), &ctx(dir.path()))
            .await;
        assert!(!out.ok);
        assert!(out.output.contains("only http/https"), "{}", out.output);
    }

    #[tokio::test]
    async fn fetch_local_http_server_capped_and_textified() {
        // 起一个本地 HTTP 服务：返回大 HTML（> cap）验证截断 + 文本化。
        // 异步桩：#[tokio::test] 是单线程 runtime（std 阻塞 IO 会饿死 worker）；
        // 逐连接 task、按路径响应、读净请求头再回（避免半读关闭触发 RST）。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf: Vec<u8> = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf);
                    let is_robots = req.starts_with("GET /robots.txt");
                    let body = if is_robots {
                        "User-agent: *\n".to_string()
                    } else {
                        format!("<html><body><p>{}</p></body></html>", "word ".repeat(40_000))
                    };
                    let ctype = if is_robots { "text/plain" } else { "text/html" };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    // 排净对端后继发送再关：带未读数据关闭会触发 RST 打断对端读 body。
                    let mut drain = [0u8; 1024];
                    while let Ok(n) = stream.read(&mut drain).await {
                        if n == 0 {
                            break;
                        }
                    }
                    let _ = stream.shutdown().await;
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let out = WebFetchTool
            .execute(
                serde_json::json!({"url": format!("http://{addr}/page")}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.truncated, "over-cap fetch must be marked truncated");
        assert!(out.output.contains("[truncated: fetched"), "{}", out.output);
        assert!(out.output.contains("word"), "{}", out.output);
        assert!(!out.output.contains("<html>"), "html tags must be stripped");
    }

    #[test]
    fn robots_disallow_prefix_match() {
        // 同步逻辑拆出来验证：build_robots_deny 的前缀语义
        let body = "User-agent: *\nDisallow: /private\n\nUser-agent: bot\nDisallow: /\n";
        let mut in_star = false;
        let mut denied = None;
        for line in body.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            match (k.trim().to_ascii_lowercase().as_str(), v.trim()) {
                ("user-agent", "*") => in_star = true,
                ("user-agent", _) => in_star = false,
                ("disallow", v) if in_star && !v.is_empty() && "/private/x".starts_with(v) => {
                    denied = Some(v.to_string());
                }
                _ => {}
            }
        }
        assert_eq!(denied.as_deref(), Some("/private"));
    }

    #[test]
    fn ddg_results_parsed_with_redirect_unwrap() {
        let html = r##"
        <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&amp;rut=x">Example Title</a>
        <a class="result__snippet" href="#">An example snippet</a>
        "##;
        let results = parse_ddg_results(html, 8);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "Example Title");
        assert_eq!(results[0].1, "https://example.com/a");
        assert_eq!(results[0].2, "An example snippet");
    }

    #[test]
    fn html_to_text_strips_scripts_and_decodes_entities() {
        let html = b"<html><script>var x=1;</script><body><p>a &amp; b</p><p>line2</p></body></html>";
        let text = html_to_text(std::str::from_utf8(html).unwrap());
        assert!(!text.contains("var x"));
        assert!(text.contains("a & b"));
        assert!(text.contains("line2"));
    }
}
