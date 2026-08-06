use serde::{Deserialize, Serialize};

/// 上游 provider 的 API 协议面。纯透传模式下决定请求/响应的格式归约
/// （Anthropic 客户端走 Anthropic provider，OpenAI 同理），不做协议转换。
///
/// serde `rename_all = "lowercase"`：`Anthropic` <-> `"anthropic"`，
/// `OpenAi` <-> `"openai"`。协议嗅探逻辑（header + 路径表）是 Task 3 的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Anthropic => "anthropic",
            Protocol::OpenAi => "openai",
        }
    }
}
