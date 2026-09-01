//! Anthropic Messages 流式客户端（task 2.2/2.3，design N5）。
//!
//! 端点可配置（D3 修订）：直连 provider（默认，[`AnthropicMessagesClient::direct_provider`]）
//! 或本地 gateway（[`AnthropicMessagesClient::gateway`]）。两者都是"Anthropic Messages
//! 流式 HTTP 端点"，对内核只是端点与凭证不同，wire protocol 完全一致。
//! 不内嵌任何 provider SDK（spec：LLM channel）。

use super::{LlmClient, LlmError, LlmRequest, LlmTurn, StopReason, StreamEvent};
use crate::message::{strip_thinking, ContentBlock, Message};
use async_trait::async_trait;
use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;

/// 鉴权风格。
#[derive(Debug, Clone)]
pub enum Auth {
    /// Anthropic 风格 `x-api-key` 头（直连 provider 与 gateway 的 claude 兼容入口均用此）。
    ApiKey(String),
    /// `Authorization: Bearer` 头。
    Bearer(String),
}

impl Auth {
    fn header(self) -> (String, String) {
        match self {
            Auth::ApiKey(k) => ("x-api-key".to_string(), k),
            Auth::Bearer(t) => ("authorization".to_string(), format!("Bearer {t}")),
        }
    }
}

pub struct AnthropicMessagesClient {
    http: reqwest::Client,
    endpoint: String,
    auth: Auth,
    anthropic_version: String,
}

impl AnthropicMessagesClient {
    /// 直连 provider（默认路径）：base_url 如 `https://api.anthropic.com` 或任意
    /// Anthropic 兼容上游；不需要运行 gateway。
    pub fn direct_provider(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::with_auth(base_url, Auth::ApiKey(api_key.into()))
    }

    /// 经可选 gateway：端点为本地 gateway（如 `http://127.0.0.1:8787`）。
    pub fn gateway(url: impl Into<String>, auth_token: impl Into<String>) -> Self {
        Self::with_auth(url, Auth::ApiKey(auth_token.into()))
    }

    pub fn with_auth(endpoint: impl Into<String>, auth: Auth) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.into(),
            auth,
            anthropic_version: "2023-06-01".to_string(),
        }
    }

    fn request_body(&self, req: &LlmRequest) -> serde_json::Value {
        serde_json::json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "system": req.system,
            "messages": req.messages.iter().map(|m: &Message| serde_json::json!({
                "role": m.role,
                "content": strip_thinking(&m.content),
            })).collect::<Vec<_>>(),
            "tools": req.tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })).collect::<Vec<_>>(),
        })
    }
}

#[async_trait]
impl LlmClient for AnthropicMessagesClient {
    async fn stream_turn(
        &self,
        req: &LlmRequest,
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<LlmTurn, LlmError> {
        let url = format!("{}/v1/messages", self.endpoint.trim_end_matches('/'));
        let (hname, hval) = self.auth.clone().header();
        let resp = self
            .http
            .post(&url)
            .header(&hname, hval)
            .header("anthropic-version", &self.anthropic_version)
            .header("accept", "text/event-stream")
            .json(&self.request_body(req))
            .send()
            .await
            .map_err(|e| LlmError::retryable(format!("request to {url} failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let code = status.as_u16();
            // 鉴权/端点配置错 → 换配置也没用，terminal；429/5xx → 可重试
            let terminal = code == 401 || code == 403 || code == 404;
            return Err(LlmError {
                terminal,
                message: format!(
                    "HTTP {status}: {}",
                    body.chars().take(500).collect::<String>()
                ),
            });
        }
        consume_sse(resp.bytes_stream(), sink).await
    }
}

fn map_stop(s: &str) -> StopReason {
    match s {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

/// SSE 字节流 →（增量事件回调，装配完成的 LlmTurn）。
/// 独立函数以便帧 fixture 直测（task 2.2），不经过 HTTP 层。
pub async fn consume_sse<S, F, E>(bytes: S, sink: &F) -> Result<LlmTurn, LlmError>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn(StreamEvent) + Sync + ?Sized,
{
    // bytes_stream() 未固定：eventsource 内部 pin 投影要求 Unpin——先 Box::pin。
    let mut es = std::pin::pin!(bytes.eventsource());
    let mut content: Vec<ContentBlock> = Vec::new();
    struct ToolBuf {
        id: String,
        name: String,
        json: String,
    }
    let mut tool_bufs: HashMap<u64, ToolBuf> = HashMap::new();
    let mut cur_text: Option<String> = None;
    let mut cur_think: Option<String> = None;
    let mut stop_reason: Option<StopReason> = None;

    while let Some(ev) = es.next().await {
        let ev = ev.map_err(|e| LlmError::terminal(format!("sse stream error: {e}")))?;
        if ev.data.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&ev.data)
            .map_err(|e| LlmError::terminal(format!("malformed sse json: {e}")))?;
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "message_start" => {}
            "content_block_start" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                let block = v.get("content_block").cloned().unwrap_or_default();
                match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "tool_use" => {
                        tool_bufs.insert(
                            idx,
                            ToolBuf {
                                id: block
                                    .get("id")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                name: block
                                    .get("name")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                // input_json_delta 分片是完整参数的增量；start 里的
                                // input 仅在流未分片（一次性完整）时非空——空对象
                                // 不预置，否则会拼成 `{}{...}` 非法 JSON。
                                json: block
                                    .get("input")
                                    .filter(|i| i.as_object().map(|o| !o.is_empty()).unwrap_or(false))
                                    .map(|i| i.to_string())
                                    .unwrap_or_default(),
                            },
                        );
                    }
                    "text" => {
                        cur_text = Some(String::new());
                    }
                    "thinking" => {
                        cur_think = Some(String::new());
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                let delta = v.get("delta").cloned().unwrap_or_default();
                match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "text_delta" => {
                        let t = delta.get("text").and_then(|x| x.as_str()).unwrap_or("");
                        if let Some(b) = cur_text.as_mut() {
                            b.push_str(t);
                        }
                        sink(StreamEvent::TextDelta(t.to_string()));
                    }
                    "thinking_delta" => {
                        let t = delta.get("thinking").and_then(|x| x.as_str()).unwrap_or("");
                        if let Some(b) = cur_think.as_mut() {
                            b.push_str(t);
                        }
                        sink(StreamEvent::ThinkingDelta(t.to_string()));
                    }
                    "input_json_delta" => {
                        // 不发事件：工具参数只有到 content_block_stop 才完整
                        if let Some(b) = tool_bufs.get_mut(&idx) {
                            b.json.push_str(
                                delta
                                    .get("partial_json")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or(""),
                            );
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                if let Some(b) = tool_bufs.remove(&idx) {
                    let input: serde_json::Value = if b.json.trim().is_empty() {
                        serde_json::Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str(&b.json)
                            .map_err(|e| LlmError::terminal(format!("tool args invalid json: {e}")))?
                    };
                    content.push(ContentBlock::ToolUse {
                        id: b.id,
                        name: b.name,
                        input,
                    });
                } else if let Some(t) = cur_text.take() {
                    content.push(ContentBlock::Text { text: t });
                } else if let Some(t) = cur_think.take() {
                    content.push(ContentBlock::Thinking { thinking: t });
                }
            }
            "message_delta" => {
                if let Some(sr) = v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                {
                    stop_reason = Some(map_stop(sr));
                }
            }
            "message_stop" => break,
            "error" => {
                // provider 在流内上报的错误：重试有意义 → 非 terminal
                let m = v
                    .pointer("/error/message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("provider reported stream error");
                return Err(LlmError::retryable(m.to_string()));
            }
            _ => {} // ping 等
        }
    }

    Ok(LlmTurn {
        content,
        stop_reason: stop_reason.unwrap_or(StopReason::EndTurn),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::sync::{Arc, Mutex};

    fn frame(event: &str, data: serde_json::Value) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    /// 录制的帧序列 fixture：文本两段 delta + 工具调用参数分片（input_json_delta）。
    fn fixture_frames() -> Vec<String> {
        vec![
            frame(
                "message_start",
                serde_json::json!({"type":"message_start","message":{"id":"msg_1"}}),
            ),
            frame(
                "content_block_start",
                serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
            ),
            frame(
                "content_block_delta",
                serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}),
            ),
            frame(
                "content_block_delta",
                serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}),
            ),
            frame(
                "content_block_stop",
                serde_json::json!({"type":"content_block_stop","index":0}),
            ),
            frame(
                "content_block_start",
                serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"bash","input":{}}}),
            ),
            frame(
                "content_block_delta",
                serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"comm"}}),
            ),
            frame(
                "content_block_delta",
                serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"and\":\"ls -la\"}"}}),
            ),
            frame(
                "content_block_stop",
                serde_json::json!({"type":"content_block_stop","index":1}),
            ),
            frame(
                "message_delta",
                serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            ),
            frame(
                "message_stop",
                serde_json::json!({"type":"message_stop"}),
            ),
        ]
    }

    #[tokio::test]
    async fn fragments_assemble_and_deltas_arrive_in_order() {
        // 跨 chunk 边界：把字节流从中间切开，帧被劈成两半
        let all = fixture_frames().join("");
        let cut = all.len() / 2;
        let s = stream::iter(vec![
            Ok::<Bytes, std::convert::Infallible>(Bytes::copy_from_slice(
                all.as_bytes()[..cut].as_ref(),
            )),
            Ok(Bytes::copy_from_slice(all.as_bytes()[cut..].as_ref())),
        ]);

        let seen: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let sink = move |e: StreamEvent| {
            seen2.lock().unwrap().push(e);
        };
        let turn = consume_sse(s, &sink).await.unwrap();

        // 增量在流中被回调（先于 consume_sse 返回），顺序保持
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                StreamEvent::TextDelta("Hello ".into()),
                StreamEvent::TextDelta("world".into())
            ]
        );
        // input_json_delta 不发事件：参数在 content_block_stop 才装配（无过早执行的可能）
        assert_eq!(turn.stop_reason, StopReason::ToolUse);
        assert_eq!(turn.content.len(), 2);
        assert!(
            matches!(&turn.content[0], ContentBlock::Text { text } if text == "Hello world"),
            "text block must be assembled from deltas"
        );
        match &turn.content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "bash");
                assert_eq!(*input, serde_json::json!({"command": "ls -la"}));
            }
            other => panic!("expected tool_use, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn direct_provider_endpoint_receives_the_request() {
        // spec「Direct provider endpoint without a gateway」：配置 provider
        // base URL + 凭证 → 请求发往该端点，全程无 gateway。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (hit_tx, hit_rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let _ = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = hit_tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = concat!(
                    "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
                );
                let _ = s.write_all(
                    format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}").as_bytes(),
                );
            }
        });
        let client = AnthropicMessagesClient::direct_provider(format!("http://{addr}"), "sk-direct");
        let req = LlmRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![Message::user_text("hi")],
            tools: vec![],
            max_tokens: 64,
        };
        let turn = client
            .stream_turn(&req, &|_: StreamEvent| {})
            .await
            .unwrap();
        assert_eq!(turn.stop_reason, StopReason::EndTurn);
        let http = hit_rx.recv().unwrap();
        // 请求确实打到了直连端点（POST /v1/messages + x-api-key）。
        assert!(http.starts_with("POST /v1/messages"), "{http}");
        assert!(http.contains("x-api-key: sk-direct"), "{http}");
    }

    #[tokio::test]
    async fn thinking_deltas_stream_immediately() {
        // spec「Deltas arrive during streaming」：thinking delta 与 text delta
        // 一样在流内即时回调（sink 先于 turn 完成被调用）。
        let frames = [
            frame("message_start", serde_json::json!({"type":"message_start"})),
            frame("content_block_start", serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}})),
            frame("content_block_delta", serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm "}})),
            frame("content_block_delta", serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"ok"}})),
            frame("content_block_stop", serde_json::json!({"type":"content_block_stop","index":0})),
            frame("message_delta", serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}})),
            frame("message_stop", serde_json::json!({"type":"message_stop"})),
        ];
        let all = frames.join("");
        let s = stream::iter(vec![Ok::<Bytes, std::convert::Infallible>(Bytes::from(all))]);
        let got: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let g2 = got.clone();
        let sink = move |e: StreamEvent| g2.lock().unwrap().push(e);
        let turn = consume_sse(s, &sink).await.unwrap();

        let evs = got.lock().unwrap();
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], StreamEvent::ThinkingDelta(t) if t == "hmm "));
        assert!(matches!(&evs[1], StreamEvent::ThinkingDelta(t) if t == "ok"));
        // 装配完成的块保留 thinking 内容。
        assert!(matches!(
            &turn.content[0],
            ContentBlock::Thinking { thinking } if thinking == "hmm ok"
        ));
    }

    #[tokio::test]
    async fn stream_error_event_is_retryable() {
        let s: stream::Iter<std::vec::IntoIter<Result<Bytes, std::io::Error>>> =
            stream::iter(vec![Ok(Bytes::from(frame(
                "error",
                serde_json::json!({"type":"error","error":{"type":"overloaded_error","message":"overloaded"}}),
            )))]);
        let err = consume_sse(s, &|_| {}).await.unwrap_err();
        assert!(!err.terminal, "provider stream errors are retryable");
        assert!(err.message.contains("overloaded"));
    }

    #[tokio::test]
    async fn request_body_strips_thinking_and_maps_tools() {
        let client = AnthropicMessagesClient::direct_provider("http://x", "k");
        let req = LlmRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![Message {
                role: crate::message::Role::Assistant,
                content: vec![
                    ContentBlock::Thinking {
                        thinking: "secret".into(),
                    },
                    ContentBlock::Text {
                        text: "visible".into(),
                    },
                ],
            }],
            tools: vec![ToolSchema {
                name: "bash".into(),
                description: "run".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }],
            max_tokens: 1234,
        };
        let body = client.request_body(&req);
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "m");
        assert_eq!(body["max_tokens"], 1234);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1, "thinking blocks stripped from history");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(body["tools"][0]["name"], "bash");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    use crate::llm::ToolSchema;
}
