//! 下游 token 鉴权中间件（spec §4.5）。
//!
//! `require_key` 以 `from_fn_with_state` 挂在 fallback 层上方，覆盖所有路由
//! （含 `/healthz`，由中间件内豁免）。`Authorization: Bearer <key>` 优先、
//! `x-api-key: <key>` 次之；缺失/未匹配 → 401，协议面由 `resolve_target` 嗅探，
//! 无法识别时默认 OpenAi。
//!
//! 安全铁律：401 message 恒为 `"invalid or missing API key"`，绝不回显呈现的
//! key，也不落日志。

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::error_response;
use crate::proto::{WireProtocol, resolve_target};
use crate::server::AppState;

/// 从 `Authorization: Bearer <key>` 或 `x-api-key: <key>` 提取下游 key。
/// Authorization 优先；Bearer 令牌为空时回退 `x-api-key`。两者均无/空 → `None`。
/// `pub(crate)`：限流中间件（rate_limit.rs）复用同一把 key 做 per-token 维度。
pub(crate) fn extract_key(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get("authorization")
        && let Ok(s) = auth.to_str()
        && let Some(rest) = s.strip_prefix("Bearer ")
        && !rest.is_empty()
    {
        return Some(rest.to_string());
    }
    if let Some(k) = headers.get("x-api-key")
        && let Ok(s) = k.to_str()
        && !s.is_empty()
    {
        return Some(s.to_string());
    }
    None
}

/// 渲染 401：协议面由 `resolve_target` 嗅探；非 `/v1` 路径 → 默认 OpenAi。
/// message 恒为通用串，绝不回显呈现的 key。
fn unauthorized(headers: &HeaderMap, path: &str) -> Response {
    let proto = resolve_target(headers, path)
        .map(|t| t.protocol)
        .unwrap_or(WireProtocol::OpenAi);
    let err_type = match proto {
        WireProtocol::Anthropic => "authentication_error",
        WireProtocol::OpenAi => "invalid_request_error",
    };
    error_response(
        proto,
        StatusCode::UNAUTHORIZED,
        err_type,
        "invalid or missing API key",
    )
}

/// 鉴权中间件。`/healthz` 豁免；`auth_token` 未配置时不校验（裸奔，启动时
/// 已 warn）；配置后要求呈现的 token 命中集合才放行。
pub async fn require_key(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // /healthz 豁免——layer 挂在 fallback 上方，故 /healthz 也过此中间件，
    // 由这里短路放行，不校验 key。
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    // debug 模式（--debug，内置 test provider）或未配置 auth_token：跳过下游
    // 鉴权（裸奔）。build_state 启动时对后者已 warn。
    if state.cfg.debug || state.auth_tokens.is_empty() {
        return next.run(req).await;
    }
    match extract_key(req.headers()) {
        Some(key) if state.auth_tokens.contains(&key) => next.run(req).await,
        _ => unauthorized(req.headers(), req.uri().path()),
    }
}
