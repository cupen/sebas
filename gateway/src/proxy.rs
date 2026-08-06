//! 透传引擎（Task 7，spec §4.3）。
//!
//! 纯透传网关核心：同协议转发，不改写协议体。`handle` 是 axum fallback
//! handler，承接 `require_key` 中间件放行后的所有非 `/healthz` 请求。
//!
//! 流程：协议嗅探 (`resolve_target`) → model 提取（buffered JSON body /
//! `/v1/models/{id}` path）→ 路由解析 (`RouteTable::resolve`) → 限流 (`Quota::check`)
//! → header 改写 + 上游 key 注入 → body 改写（model rename）或流式透传 →
//! 上游响应原样回传（SSE 逐 chunk flush，非 SSE 缓冲）。
//!
//! 纯函数拆分以便单测：`filtered_request_headers` / `filtered_response_headers` /
//! `rename_model_in_body`。`handle` 串联这些 + 路由/限流/客户端，映射错误。
//!
//! 安全铁律：下游 key 绝不出现在转发请求里；上游 key 只注入到 outbound
//! header，不落日志/响应；5xx 用通用 message。

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;

use crate::auth::KeyIdentity;
use crate::config::ProviderConfig;
use crate::error::error_response;
use crate::proto::{Protocol, resolve_target};
use crate::quota::QuotaVerdict;
use crate::routing::{RouteError, extract_model_from_body, extract_model_from_path};
use crate::server::AppState;

/// hop-by-hop + 下游 auth header：请求侧剥离集合（spec §4.3）。
/// `host` 由 reqwest 按目标 URL 重算；`content-length` 按新 body 重算；
/// `authorization`/`x-api-key` 是下游 key，必须剥离后注入上游 key。
const REQUEST_STRIP_HEADERS: &[&str] = &[
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "authorization",
    "x-api-key",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
];

/// 响应侧剥离集合。`content-length` 由 axum 按新 body 重算；其余是 hop-by-hop。
const RESPONSE_STRIP_HEADERS: &[&str] = &[
    "connection",
    "transfer-encoding",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    "content-length",
];

/// 改写请求 header：剥离 hop-by-hop + 下游 key，按协议注入上游 key。
/// 业务 header（`anthropic-version` / `anthropic-beta` / `content-type` / 自定义）
/// 原样透传。HeaderName 与 `&str` 比较已 case-insensitive（http crate 行为）。
pub fn filtered_request_headers(src: &HeaderMap, proto: Protocol, upstream_key: &str) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in src.iter() {
        if REQUEST_STRIP_HEADERS.iter().any(|s| name == *s) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    inject_upstream_auth(&mut out, proto, upstream_key);
    out
}

/// 改写响应 header：剥离 hop-by-hop + `content-length`（axum 按新 body 重算）。
/// 其余（`content-type` / `retry-after` / 业务 header）原样透传。
pub fn filtered_response_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in src.iter() {
        if RESPONSE_STRIP_HEADERS.iter().any(|s| name == *s) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

/// 按 `RouteDecision.upstream_model` 重写 buffered JSON body 的 `model` 字段。
/// 返回 `Some(new_bytes)` 当 body 是 JSON 对象且含字符串 `model` 字段且与
/// `upstream_model` 不同；否则 `None`（非 JSON / 非 object / `model` 非字符串 /
/// 无 `model` 字段 / 已等于目标）—— 调用方原样转发。
///
/// 解析失败绝不阻断透传：rename 是 best-effort。其它字段保留（serde_json::Value
/// 会按字母序重排 key，但不丢字段）。
pub fn rename_model_in_body(body: &Bytes, upstream_model: &str) -> Option<Bytes> {
    let mut v: serde_json::Value = serde_json::from_slice(body.as_ref()).ok()?;
    let obj = v.as_object_mut()?;
    let m = obj.get("model")?.as_str()?;
    if m == upstream_model {
        return None;
    }
    obj.insert(
        "model".to_string(),
        serde_json::Value::String(upstream_model.to_string()),
    );
    serde_json::to_vec(&v).ok().map(Bytes::from)
}

/// 按 protocol 注入上游 auth header（`x-api-key` / `Authorization: Bearer`）。
/// 用 `insert` 覆盖（取代 append）——上游 key 唯一来源，单值即可。
fn inject_upstream_auth(out: &mut HeaderMap, proto: Protocol, upstream_key: &str) {
    match proto {
        Protocol::Anthropic => {
            out.insert(
                "x-api-key",
                upstream_key
                    .parse()
                    .expect("upstream key must be a valid header value"),
            );
        }
        Protocol::OpenAi => {
            let val = format!("Bearer {upstream_key}");
            out.insert(
                "authorization",
                val.parse()
                    .expect("Bearer <upstream_key> must be a valid header value"),
            );
        }
    }
}

/// 判断 content-type 是否为 JSON（用于决定是否 buffer body 做 model 提取）。
/// 容忍 `application/json; charset=utf-8` 等变体——按 "json" 子串判定。
fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("json"))
}

/// 判断响应 content-type 是否为 SSE（`text/event-stream`）。
fn is_sse_content_type(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/event-stream"))
}

/// 判断 method 是否需要 buffer body（POST/PUT/PATCH）。GET/DELETE 流式不缓冲。
fn is_buffer_method(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH)
}

/// 把 `RouteError` 映射成协议面错误响应。状态码与 err_type 按 brief 契约：
/// `ModelNotAllowed` → 403 `permission_error`；`ProtocolMismatch` → 400
/// `invalid_request_error`；`NoRoute` → 502 `no_route`。message 通用，不含 key。
fn route_error_response(proto: Protocol, err: &RouteError) -> Response {
    let (status, err_type, message): (StatusCode, &str, String) = match err {
        RouteError::ModelNotAllowed => (
            StatusCode::FORBIDDEN,
            "permission_error",
            "model not allowed by this key".to_string(),
        ),
        RouteError::ProtocolMismatch { provider } => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            // provider 名是配置项，非密钥；可入 message 帮助排障。
            format!("provider '{provider}' does not speak the request protocol"),
        ),
        RouteError::NoRoute => (
            StatusCode::BAD_GATEWAY,
            "no_route",
            "no provider route matched the request".to_string(),
        ),
    };
    error_response(proto, status, err_type, &message)
}

/// 429 限流响应：按 protocol 渲染错误体 + `Retry-After` header。
/// `reason` 串来自 `Quota::check`（`REASON_RPM` / `REASON_DAILY_TOKEN_QUOTA`），
/// 直接作为 err_type 入协议面错误体。
fn quota_denied_response(proto: Protocol, retry_after_secs: u64, reason: &str) -> Response {
    let mut resp = error_response(
        proto,
        StatusCode::TOO_MANY_REQUESTS,
        reason,
        "rate limit or quota exceeded",
    );
    let _ = resp.headers_mut().insert(
        "retry-after",
        retry_after_secs
            .to_string()
            .parse()
            .expect("retry_after_secs as ascii digits is a valid header value"),
    );
    resp
}

/// 网关自身 5xx（上游不可达 / 配置缺失）。通用 message，不含 key/内部细节。
fn upstream_error_response(proto: Protocol, message: &str) -> Response {
    error_response(proto, StatusCode::BAD_GATEWAY, "upstream_error", message)
}

/// 透传 fallback handler。承接 `require_key` 放行后的所有非 `/healthz` 请求。
///
/// 错误映射（brief 契约）：
/// - 非 `/v1` 路径 → 404（默认 OpenAI 格式）
/// - quota Deny → 429 + `Retry-After`
/// - `ModelNotAllowed` → 403；`ProtocolMismatch` → 400；`NoRoute` → 502
/// - 上游连接/读取失败 → 502 `upstream_error`
/// - 上游 key 缺失（provider 未配 api_key_env/api_key）→ 502 `upstream_error`
/// - body 超 `max_body_bytes` → 413
///
/// 上游响应（含 4xx/5xx + 上游 retry-after）原样透传，status/body/headers 不改。
pub async fn handle(State(state): State<AppState>, req: Request) -> Response {
    // 1. 协议嗅探 + 路径解析。非 /v1 → 404（默认 OpenAI 协议面，与 auth.rs 一致）。
    let target = match resolve_target(req.headers(), req.uri().path()) {
        Some(t) => t,
        None => {
            return error_response(
                Protocol::OpenAi,
                StatusCode::NOT_FOUND,
                "not_found",
                "path is not under /v1",
            );
        }
    };
    let proto = target.protocol;

    // 2. 取 KeyIdentity（require_key 中间件已注入）。缺失 → 500（防御性，正常不触发）。
    let identity = match req.extensions().get::<KeyIdentity>().cloned() {
        Some(i) => i,
        None => {
            return error_response(
                proto,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "gateway state missing key identity",
            );
        }
    };

    // 3. 限流/配额判定。Deny → 429 + Retry-After（网关自身 429，区别于上游 429）。
    match state.quota.check(&identity) {
        QuotaVerdict::Allow => {}
        QuotaVerdict::Deny {
            retry_after_secs,
            reason,
        } => {
            return quota_denied_response(proto, retry_after_secs, reason);
        }
    }

    // 4. 拆解 Request：method/uri/headers/body。
    let method = req.method().clone();
    let uri = req.uri().clone();
    let req_headers = req.headers().clone();
    // 用 Option 包 body，便于「buffer 路径 take 走，stream 路径保留到 wrap_stream」
    // 的条件移动——避免编译器对「if 分支 move 而 else 不 move」的报错。
    let mut body_opt = Some(req.into_body());

    // 5. body 策略：buffer-method + JSON content-type → 缓冲（超限 413）；其余流式。
    //    缓冲路径用于 model 提取与 rename；流式路径 model 仅从 path 取。
    let buffered_bytes: Option<Bytes> =
        if is_buffer_method(&method) && is_json_content_type(&req_headers) {
            let body = body_opt.take().expect("body present before take");
            match to_bytes(body, state.cfg.max_body_bytes as usize).await {
                Ok(b) => Some(b),
                Err(e) => {
                    // 长度超限 → 413；其它 IO/解析错误 → 400。
                    use std::error::Error as _;
                    let is_length_limit = e
                        .source()
                        .is_some_and(|s| s.is::<http_body_util::LengthLimitError>());
                    let (status, err_type, message) = if is_length_limit {
                        (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "request_too_large",
                            "request body exceeds gateway max_body_bytes",
                        )
                    } else {
                        (
                            StatusCode::BAD_REQUEST,
                            "invalid_request_error",
                            "failed to read request body",
                        )
                    };
                    return error_response(proto, status, err_type, message);
                }
            }
        } else {
            None
        };

    // 6. model 提取：buffered JSON body 优先；否则从 `/v1/models/{id}` path 取。
    let model: Option<String> = buffered_bytes
        .as_ref()
        .and_then(extract_model_from_body)
        .or_else(|| extract_model_from_path(&target.path));

    // 7. 路由解析。错误按 brief 映射；成功拿到 provider + upstream_model。
    let decision = match state
        .table
        .resolve(model.as_deref(), proto, Some(&identity.config))
    {
        Ok(d) => d,
        Err(e) => return route_error_response(proto, &e),
    };

    // 8. 取上游 provider 配置 + 上游 key。任一缺失（配置不一致）→ 502 防御性错误。
    let provider_cfg: &ProviderConfig = match state.cfg.providers.get(&decision.provider) {
        Some(p) => p,
        None => {
            return upstream_error_response(
                proto,
                "resolved provider not found in config (gateway state inconsistent)",
            );
        }
    };
    let upstream_key = match state.api_keys.get(&decision.provider) {
        Some(k) => k,
        None => {
            return upstream_error_response(
                proto,
                "upstream api key not resolved for provider (check api_key_env/api_key)",
            );
        }
    };

    // 9. 构造上游 URL：base_url + target.path + preserved query string。
    let upstream_url = build_upstream_url(&provider_cfg.base_url, &target.path, &uri);

    // 10. 改写请求 header（剥离 hop-by-hop + 下游 key，注入上游 key）。
    let out_headers = filtered_request_headers(&req_headers, proto, upstream_key);

    // 11. 构造上游请求 body：buffered（可能 rename）或流式 wrap_stream。
    //     仅当 upstream_model 存在且与客户端原 model 不同时才 rename（brief 契约）。
    let out_body: reqwest::Body = if let Some(bytes) = buffered_bytes {
        let final_bytes = match &decision.upstream_model {
            Some(um) if Some(um.as_str()) != model.as_deref() => {
                rename_model_in_body(&bytes, um).unwrap_or_else(|| bytes.clone())
            }
            _ => bytes.clone(),
        };
        reqwest::Body::from(final_bytes)
    } else {
        let body = body_opt.take().expect("body present for streaming path");
        // BodyDataStream (axum) → reqwest::Body::wrap_stream。
        // axum_core::Error: StdError + Send + Sync + 'static → Into<BoxError> 走 blanket From。
        reqwest::Body::wrap_stream(body.into_data_stream())
    };

    // 12. 发请求。send 失败（连接/读取超时/拒绝）→ 502 upstream_error。
    let upstream_resp = match state
        .client
        .request(method, &upstream_url)
        .headers(out_headers)
        .body(out_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, %upstream_url, "upstream request failed");
            return upstream_error_response(proto, "failed to reach upstream provider");
        }
    };

    // 13. 构造响应：status 原样；header 剥离 hop-by-hop；body 按 SSE/非 SSE 分流。
    let status = upstream_resp.status();
    let is_sse = is_sse_content_type(upstream_resp.headers());
    let resp_headers = filtered_response_headers(upstream_resp.headers());

    // 用 `Response::new(body)` 起手（200 + 空 header），再覆盖 status/headers，
    // 避开 `Builder::headers_mut` 返回 `Option<&mut HeaderMap>` 的繁琐。
    let mut resp = if is_sse {
        // SSE：逐 chunk flush 不缓冲。Task 8 在此插 usage tee（per-chunk 边界）。
        let stream = sse_passthrough_stream(upstream_resp);
        Response::new(Body::from_stream(stream))
    } else {
        // 非 SSE：缓冲回传。读取失败 → 502 upstream_error。
        match upstream_resp.bytes().await {
            Ok(b) => Response::new(Body::from(b)),
            Err(e) => {
                tracing::warn!(error = %e, %upstream_url, "upstream body read failed");
                return upstream_error_response(proto, "failed to read upstream response body");
            }
        }
    };
    *resp.status_mut() = status;
    *resp.headers_mut() = resp_headers;
    resp
}

/// 构造上游 URL：`base_url` + `target.path` + 原始 query string。
/// `base_url` 末尾可能带 `/`，`target.path` 必以 `/v1` 开头——拼接前先剥尾斜杠。
fn build_upstream_url(base_url: &str, target_path: &str, uri: &axum::http::Uri) -> String {
    let trimmed_base = base_url.trim_end_matches('/');
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    format!("{trimmed_base}{target_path}{query}")
}

/// SSE 透传流：把 reqwest response 的 bytes_stream 转成 axum `Body::from_stream`
/// 可消费的 `Stream<Item = Result<Bytes, BoxError>>`。
///
/// 把流单独函数化是 Task 8 的 SPI 边界：未来在此函数内对每 chunk 做增量解析
/// （usage tee），不影响 `handle` 主流程。当前实现是纯字节透传——`map_err`
/// 把 `reqwest::Error` 装箱为 `BoxError` 以满足 axum `Body::from_stream` 的
/// `Error: Into<BoxError>` 约束。
fn sse_passthrough_stream(
    upstream_resp: reqwest::Response,
) -> impl futures_core::Stream<Item = Result<Bytes, Box<dyn std::error::Error + Send + Sync>>> {
    use futures_util::TryStreamExt;
    upstream_resp
        .bytes_stream()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr_multi(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (n, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(n.as_bytes()).unwrap(),
                (*v).parse().unwrap(),
            );
        }
        h
    }

    // ---------------- 1. Anthropic 请求剥离 + x-api-key 注入（含下游 key 不泄漏） ----------------

    #[test]
    fn anthropic_request_strips_and_injects_x_api_key_no_downstream_leak() {
        let src = hdr_multi(&[
            ("host", "gateway.local"),
            ("connection", "keep-alive"),
            ("content-length", "42"),
            ("transfer-encoding", "chunked"),
            ("authorization", "Bearer sk-downstream-secret"),
            ("x-api-key", "sk-downstream-secret"),
            ("keep-alive", "timeout=30"),
            ("te", "trailers"),
            ("trailer", "x-foo"),
            ("upgrade", "h2c"),
            // 业务 header 原样透传
            ("anthropic-version", "2023-06-01"),
            ("anthropic-beta", "prompt-caching-2024-07-31"),
            ("content-type", "application/json"),
            ("x-custom", "hello"),
        ]);
        let out = filtered_request_headers(&src, Protocol::Anthropic, "sk-upstream-anthropic");

        // 注入的上游 key
        assert_eq!(out.get("x-api-key").unwrap(), "sk-upstream-anthropic");
        // 业务 header 透传
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(
            out.get("anthropic-beta").unwrap(),
            "prompt-caching-2024-07-31"
        );
        assert_eq!(out.get("content-type").unwrap(), "application/json");
        assert_eq!(out.get("x-custom").unwrap(), "hello");
        // hop-by-hop + 下游 key 全部剥离（x-api-key 已被注入上游值，单独排除）
        for stripped in [
            "host",
            "connection",
            "content-length",
            "transfer-encoding",
            "authorization",
            "keep-alive",
            "te",
            "trailer",
            "upgrade",
        ] {
            assert!(
                out.get(stripped).is_none(),
                "{stripped} should be stripped, found {:?}",
                out.get(stripped)
            );
        }
        assert!(
            !out.contains_key("authorization"),
            "authorization must be stripped for Anthropic"
        );
        // 下游 key 绝不出现在任何 header 值里
        for (_, v) in out.iter() {
            let s = v.to_str().expect("header value is ascii");
            assert!(
                !s.contains("sk-downstream-secret"),
                "downstream key leaked in header value: {s}"
            );
        }
    }

    // ---------------- 2. OpenAI Bearer 注入 ----------------

    #[test]
    fn openai_request_injects_bearer_and_strips_x_api_key() {
        let src = hdr_multi(&[
            ("authorization", "Bearer sk-downstream-openai"),
            ("x-api-key", "sk-downstream-openai"),
            ("content-type", "application/json"),
            ("x-request-id", "abc-123"),
        ]);
        let out = filtered_request_headers(&src, Protocol::OpenAi, "sk-upstream-openai");

        assert_eq!(
            out.get("authorization").unwrap(),
            "Bearer sk-upstream-openai"
        );
        // x-api-key 是 Anthropic 协议面 header；OpenAI 侧也不应残留下游值
        assert!(
            !out.contains_key("x-api-key"),
            "x-api-key should be stripped (no upstream value for OpenAi)"
        );
        // 业务 header 透传
        assert_eq!(out.get("content-type").unwrap(), "application/json");
        assert_eq!(out.get("x-request-id").unwrap(), "abc-123");
        // 下游 key 不泄漏
        for (_, v) in out.iter() {
            let s = v.to_str().expect("ascii");
            assert!(!s.contains("sk-downstream-openai"), "leak: {s}");
        }
    }

    // ---------------- 3. 响应 hop-by-hop 剥离 ----------------

    #[test]
    fn response_strips_hop_by_hop_and_content_length_keeps_others() {
        let src = hdr_multi(&[
            ("connection", "keep-alive"),
            ("transfer-encoding", "chunked"),
            ("keep-alive", "timeout=30"),
            ("te", "trailers"),
            ("trailer", "x-foo"),
            ("upgrade", "h2c"),
            ("content-length", "1234"),
            // 透传：业务 header / retry-after / content-type
            ("content-type", "text/event-stream"),
            ("retry-after", "30"),
            ("x-request-id", "upstream-abc"),
        ]);
        let out = filtered_response_headers(&src);

        for stripped in [
            "connection",
            "transfer-encoding",
            "keep-alive",
            "te",
            "trailer",
            "upgrade",
            "content-length",
        ] {
            assert!(
                out.get(stripped).is_none(),
                "{stripped} should be stripped, found {:?}",
                out.get(stripped)
            );
        }
        // 透传保留
        assert_eq!(out.get("content-type").unwrap(), "text/event-stream");
        assert_eq!(out.get("retry-after").unwrap(), "30");
        assert_eq!(out.get("x-request-id").unwrap(), "upstream-abc");
    }

    // ---------------- 4. rename 改写 JSON 且保留其它字段；非 JSON 原样 ----------------

    #[test]
    fn rename_model_rewrites_json_preserving_other_fields_non_json_unchanged() {
        // 正常：JSON object + model 字段 → 改写 model，保留 foo
        let body = Bytes::from(r#"{"model":"claude-sonnet","foo":"bar"}"#);
        let renamed = rename_model_in_body(&body, "anthropic.claude-sonnet-4")
            .expect("should rewrite valid JSON with model field");
        let v: serde_json::Value =
            serde_json::from_slice(renamed.as_ref()).expect("renamed is valid JSON");
        assert_eq!(v["model"], "anthropic.claude-sonnet-4");
        assert_eq!(v["foo"], "bar");

        // 非 JSON → None（原样转发）
        assert_eq!(rename_model_in_body(&Bytes::from("not json"), "x"), None);

        // JSON 但无 model 字段 → None（无 field 可改写）
        assert_eq!(
            rename_model_in_body(&Bytes::from(r#"{"foo":"bar"}"#), "x"),
            None,
            "no model field → None"
        );

        // JSON 但 model 非字符串（数字）→ None
        assert_eq!(
            rename_model_in_body(&Bytes::from(r#"{"model":42}"#), "x"),
            None,
            "non-string model → None"
        );

        // model 已等于 upstream_model → None（无需改写）
        assert_eq!(
            rename_model_in_body(&Bytes::from(r#"{"model":"x"}"#), "x"),
            None,
            "model already equals upstream → no rewrite"
        );

        // 嵌套对象里的 model 字段不应被误改（只改顶层 model）
        let body = Bytes::from(r#"{"model":"a","inner":{"model":"b"}}"#);
        let renamed = rename_model_in_body(&body, "renamed").expect("rewrite top-level model");
        let v: serde_json::Value = serde_json::from_slice(renamed.as_ref()).expect("valid JSON");
        assert_eq!(v["model"], "renamed");
        assert_eq!(v["inner"]["model"], "b", "nested model must be untouched");
    }
}
