//! 下游 key 鉴权中间件（Task 5，spec §4.5）。
//!
//! `require_key` 以 `from_fn_with_state` 挂在 fallback 层上方，覆盖所有路由
//! （含 `/healthz`，由中间件内豁免）。`Authorization: Bearer <key>` 优先、
//! `x-api-key: <key>` 次之；缺失/未匹配 → 401，协议面由 `resolve_target` 嗅探，
//! 无法识别时默认 OpenAi。鉴权通过后把 `KeyIdentity` 挂入 request extensions，
//! 供 Task 6（quota）/ Task 7（proxy）/ Task 8（usage）读取。
//!
//! 安全铁律：401 message 恒为 `"invalid or missing API key"`，绝不回显呈现的
//! key，也不落日志。`KeyIdentity` 只在 extensions 内流转，不进错误响应。

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::KeyConfig;
use crate::error::error_response;
use crate::proto::{Protocol, resolve_target};
use crate::server::AppState;

/// 鉴权通过后挂入 request extensions 的身份。后续中间件/handler 通过
/// `req.extension::<KeyIdentity>()` 取 key 的配置（rpm/配额/allow_models/...）。
#[derive(Debug, Clone)]
pub struct KeyIdentity {
    pub config: KeyConfig,
}

/// 从 `Authorization: Bearer <key>` 或 `x-api-key: <key>` 提取下游 key。
/// Authorization 优先；Bearer 令牌为空时回退 `x-api-key`。两者均无/空 → `None`。
fn extract_key(headers: &HeaderMap) -> Option<String> {
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
        .unwrap_or(Protocol::OpenAi);
    let err_type = match proto {
        Protocol::Anthropic => "authentication_error",
        Protocol::OpenAi => "invalid_request_error",
    };
    error_response(
        proto,
        StatusCode::UNAUTHORIZED,
        err_type,
        "invalid or missing API key",
    )
}

/// 鉴权中间件。`/healthz` 豁免；其余路径要求合法下游 key，通过则挂
/// `KeyIdentity` 到 extensions 后放行。
pub async fn require_key(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // /healthz 豁免——layer 挂在 fallback 上方，故 /healthz 也过此中间件，
    // 由这里短路放行，不校验 key。
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    // debug 模式（--debug）：内置 test provider 仅用于测试，跳过下游鉴权，
    // key 可留空。仍挂一个默认 KeyIdentity 供 proxy 走正常流程。
    if state.cfg.debug {
        let mut req = req;
        req.extensions_mut().insert(KeyIdentity {
            config: KeyConfig {
                key: String::new(),
                key_env: None,
                name: "debug".into(),
                rpm: None,
                daily_token_quota: None,
                allow_models: Vec::new(),
                default_provider: None,
            },
        });
        return next.run(req).await;
    }
    match extract_key(req.headers()) {
        Some(key) => match state.keys.get(&key) {
            Some(cfg) => {
                let mut req = req;
                req.extensions_mut().insert(KeyIdentity {
                    config: cfg.clone(),
                });
                next.run(req).await
            }
            None => unauthorized(req.headers(), req.uri().path()),
        },
        None => unauthorized(req.headers(), req.uri().path()),
    }
}
