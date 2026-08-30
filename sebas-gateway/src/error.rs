use thiserror::Error;

/// 网关自身错误。上游错误（status+body）由透传引擎原样回传，不经此 enum；
/// 此处只表达网关侧：配置、IO、鉴权、路由解析失败，以及未来透传引擎
/// 抛出的上游不可达/协议不匹配等。
///
/// 任何 key 值都不落错误信息；`api_key_env` 相关错误只含 env 变量名。
#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("upstream error: {0}")]
    Upstream(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("routing error: {0}")]
    Routing(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;

/// 按协议面把网关自身错误渲染成对应风格错误响应体（见 openspec/specs/gateway-core/spec.md）：
/// - Anthropic: `{"type":"error","error":{"type":..,"message":..}}`
/// - OpenAI:    `{"error":{"message":..,"type":..,"code":null}}`
///
/// `status` 由调用方按错误语义给出（401 鉴权 / 400 协议不匹配 / 502 无路由 …）。
pub fn error_response(
    proto: crate::proto::WireProtocol,
    status: axum::http::StatusCode,
    err_type: &str,
    message: &str,
) -> axum::response::Response {
    let body = match proto {
        crate::proto::WireProtocol::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": err_type, "message": message }
        }),
        crate::proto::WireProtocol::OpenAi => serde_json::json!({
            "error": { "message": message, "type": err_type, "code": null }
        }),
    };
    axum::response::Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("error response parts are statically valid")
}
