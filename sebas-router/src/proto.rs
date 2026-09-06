use serde::{Deserialize, Serialize};

/// 上游 provider 的 API 协议面。纯透传模式下决定请求/响应的格式归约
/// （Anthropic 客户端走 Anthropic provider，OpenAI 同理），不做协议转换。
///
/// OpenAI 家族拆两档：`OpenAiChat`（chat completions 及其余 OpenAI 端点）
/// 与 `OpenAiResponses`（Responses API）——两者可指向 provider 的不同
/// base_url 槽位（见 `ProviderConfig::url_for`），wire 格式仍同族。
///
/// Renamed from `Protocol` to disambiguate from
/// `sebas_acp::claude::AgentProtocol` (which carries the same meaning but at the
/// agent→upstream seam, not the router→upstream seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireProtocol {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
}

impl WireProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            WireProtocol::Anthropic => "anthropic",
            WireProtocol::OpenAiChat => "openai_chat",
            WireProtocol::OpenAiResponses => "openai_responses",
        }
    }

    /// OpenAI 家族（chat / responses）共用的判定：鉴权、错误形状、usage
    /// 提取在同族两档间一致。
    pub fn is_openai_family(self) -> bool {
        matches!(self, WireProtocol::OpenAiChat | WireProtocol::OpenAiResponses)
    }
}

/// 协议嗅探结果：解析出的协议 + 剥离显式前缀后的 bare `/v1/...` 路径。
/// `path` 恒以 `/v1` 开头（段边界），由 `resolve_target` 保证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub protocol: WireProtocol,
    pub path: String,
}

/// Anthropic 专属路径表（见 openspec/specs/router-core/spec.md）。段边界感知匹配：
/// `/v1/messages` 命中 `/v1/messages` 与 `/v1/messages/x`，不命中 `/v1/messagesXYZ`。
const ANTHROPIC_PATHS: &[&str] = &["/v1/messages"];

/// OpenAI Responses 专属路径表。独立于 chat 槽位路由（`/v1/responses` 及
/// 其子路径打 `base_url_openai_responses`）。
const OPENAI_RESPONSES_PATHS: &[&str] = &["/v1/responses"];

/// OpenAI chat-completions 专属路径表（见 openspec/specs/router-core/spec.md）。
/// 碰撞路径（`/v1/models`、`/v1/files`、`/v1/skills`）刻意不入表，由
/// `anthropic-version` header 仲裁。
///
/// ⚠️ **仅外部 OpenAI 客户端使用** —— sebas 自身走 Router 模式时，
/// agent 只发 Anthropic 协议，本表对 sebas→router→upstream 路径不可见。
/// 见 openspec/specs/provider-management/spec.md。
const OPENAI_CHAT_PATHS: &[&str] = &[
    "/v1/chat/completions",
    "/v1/embeddings",
    "/v1/moderations",
    "/v1/images",
    "/v1/audio",
    "/v1/videos",
    "/v1/uploads",
    "/v1/batches",
    "/v1/fine_tuning",
    "/v1/assistants",
    "/v1/threads",
    "/v1/vector_stores",
    "/v1/evals",
    "/v1/containers",
    "/v1/conversations",
    "/v1/chatkit",
    "/v1/realtime",
    "/v1/organization",
    "/v1/projects",
    "/v1/completions",
    "/v1/content_provenance_checks",
];

/// 段边界感知前缀匹配：`path == entry` 或 `path` 以 `entry + "/"` 开头。
/// 避免 `/v1/messages` 误命中 `/v1/messagesXYZ`。
fn path_matches_entry(path: &str, entry: &str) -> bool {
    if path == entry {
        return true;
    }
    // entry 是 path 的前缀，且紧随其后必须是 `/`（段分隔符）
    path.len() > entry.len() && path.starts_with(entry) && path.as_bytes()[entry.len()] == b'/'
}

/// 显式前缀挂载（`/anthropic/`、`/openai/`）——返回强制协议与剥离后的
/// bare 路径（`/v1/...`）。段边界：`/anthropic/v1` 命中，`/anthropicfoo` 不命中。
/// `/openai/` 强制 chat 档；Responses 无独立前缀（`/v1/responses` 路径本身
/// 无歧义，嗅探即可命中）。
fn explicit_prefix(path: &str) -> Option<(WireProtocol, &str)> {
    if let Some(rest) = path.strip_prefix("/anthropic")
        && (rest.starts_with('/') || rest.is_empty())
    {
        return Some((WireProtocol::Anthropic, rest));
    }
    if let Some(rest) = path.strip_prefix("/openai")
        && (rest.starts_with('/') || rest.is_empty())
    {
        return Some((WireProtocol::OpenAiChat, rest));
    }
    None
}

/// 判断路径是否落在 `/v1` 命名空间（段边界：`/v1` 或 `/v1/...`，非 `/v1foo`）。
fn is_under_v1(path: &str) -> bool {
    path == "/v1" || path.starts_with("/v1/")
}

/// 协议嗅探（见 openspec/specs/router-core/spec.md）。优先级（高 → 低）：
/// 1. 显式前缀 `/anthropic/`、`/openai/`（强制协议）
/// 2. Anthropic 专属路径表（`/v1/messages`）
/// 3. OpenAI Responses 专属路径表（`/v1/responses`）
/// 4. OpenAI chat 专属路径表
/// 5. `anthropic-version` header
/// 6. 默认 OpenAiChat
///
/// `path` 可带显式前缀（裸 `uri_path`）或 bare `/v1/...`，两种都能识别。
pub fn sniff(headers: &axum::http::HeaderMap, path: &str) -> WireProtocol {
    if let Some((proto, _)) = explicit_prefix(path) {
        return proto;
    }
    if ANTHROPIC_PATHS.iter().any(|e| path_matches_entry(path, e)) {
        return WireProtocol::Anthropic;
    }
    if OPENAI_RESPONSES_PATHS
        .iter()
        .any(|e| path_matches_entry(path, e))
    {
        return WireProtocol::OpenAiResponses;
    }
    if OPENAI_CHAT_PATHS.iter().any(|e| path_matches_entry(path, e)) {
        return WireProtocol::OpenAiChat;
    }
    if headers.contains_key("anthropic-version") {
        return WireProtocol::Anthropic;
    }
    WireProtocol::OpenAiChat
}

/// 解析嗅探目标：剥离显式前缀得到 bare `/v1/...` 路径 + 嗅探协议。
/// 非 `/v1` 路径 → `None`（proxy 映 404）。
///
/// 显式前缀存在时强制协议（即便 bare 路径落在对方路径表），否则交 `sniff` 仲裁。
pub fn resolve_target(headers: &axum::http::HeaderMap, uri_path: &str) -> Option<Target> {
    let (protocol, bare_path) = match explicit_prefix(uri_path) {
        Some((proto, rest)) => (proto, rest),
        None => (sniff(headers, uri_path), uri_path),
    };
    if !is_under_v1(bare_path) {
        return None;
    }
    Some(Target {
        protocol,
        path: bare_path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs_with(version: Option<&str>) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        if let Some(v) = version {
            h.insert("anthropic-version", v.parse().unwrap());
        }
        h
    }
    fn no_hdrs() -> axum::http::HeaderMap {
        axum::http::HeaderMap::new()
    }

    #[test]
    fn explicit_prefix_strips_and_forces_protocol() {
        // /anthropic/ forces Anthropic even on an OpenAI-specific path
        let t = resolve_target(&no_hdrs(), "/anthropic/v1/chat/completions").unwrap();
        assert_eq!(t.protocol, WireProtocol::Anthropic);
        assert_eq!(t.path, "/v1/chat/completions");

        // /openai/ forces OpenAiChat even on the Anthropic-specific /v1/messages
        let t = resolve_target(&no_hdrs(), "/openai/v1/messages").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAiChat);
        assert_eq!(t.path, "/v1/messages");

        // sniff agrees with the forced protocol
        assert_eq!(
            sniff(&no_hdrs(), "/anthropic/v1/chat/completions"),
            WireProtocol::Anthropic
        );
        assert_eq!(
            sniff(&no_hdrs(), "/openai/v1/messages"),
            WireProtocol::OpenAiChat
        );
    }

    #[test]
    fn messages_family_is_anthropic_without_header() {
        // No explicit prefix, no anthropic-version header → Anthropic by path table
        assert_eq!(sniff(&no_hdrs(), "/v1/messages"), WireProtocol::Anthropic);
        assert_eq!(
            sniff(&no_hdrs(), "/v1/messages/abc"),
            WireProtocol::Anthropic
        );

        let t = resolve_target(&no_hdrs(), "/v1/messages").unwrap();
        assert_eq!(t.protocol, WireProtocol::Anthropic);
        assert_eq!(t.path, "/v1/messages");
    }

    #[test]
    fn responses_family_is_its_own_protocol() {
        // /v1/responses and subpaths sniff as OpenAiResponses, distinct from chat
        assert_eq!(
            sniff(&no_hdrs(), "/v1/responses"),
            WireProtocol::OpenAiResponses
        );
        assert_eq!(
            sniff(&no_hdrs(), "/v1/responses/resp_abc"),
            WireProtocol::OpenAiResponses
        );
        let t = resolve_target(&no_hdrs(), "/v1/responses").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAiResponses);
        assert_eq!(t.path, "/v1/responses");

        // chat path stays chat — no bleed between the two openai slots
        assert_eq!(
            sniff(&no_hdrs(), "/v1/chat/completions"),
            WireProtocol::OpenAiChat
        );
        // segment boundary: /v1/responsesXYZ is not in the responses table
        assert_ne!(
            sniff(&no_hdrs(), "/v1/responsesXYZ"),
            WireProtocol::OpenAiResponses
        );
    }

    #[test]
    fn openai_specific_paths_detected() {
        // Representative slice of the OpenAI chat path table; no header, no prefix
        for p in [
            "/v1/chat/completions",
            "/v1/embeddings",
            "/v1/moderations",
            "/v1/images/generations",
            "/v1/audio/transcriptions",
            "/v1/videos",
            "/v1/uploads",
            "/v1/batches",
            "/v1/fine_tuning",
            "/v1/assistants",
            "/v1/threads/thread_abc",
            "/v1/vector_stores",
            "/v1/evals",
            "/v1/containers",
            "/v1/conversations",
            "/v1/chatkit",
            "/v1/realtime",
            "/v1/organization",
            "/v1/projects",
            "/v1/completions",
            "/v1/content_provenance_checks",
        ] {
            assert_eq!(
                sniff(&no_hdrs(), p),
                WireProtocol::OpenAiChat,
                "path {p} should be OpenAiChat"
            );
        }

        let t = resolve_target(&no_hdrs(), "/v1/chat/completions").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAiChat);
        assert_eq!(t.path, "/v1/chat/completions");
    }

    #[test]
    fn collision_path_arbitrated_by_header_both_directions() {
        // /v1/models, /v1/files, /v1/skills are collision paths — not in any table.
        // anthropic-version header → Anthropic; absent → default OpenAiChat.
        for p in ["/v1/models", "/v1/files", "/v1/skills"] {
            assert_eq!(
                sniff(&hdrs_with(Some("2023-06-01")), p),
                WireProtocol::Anthropic,
                "path {p} + anthropic-version → Anthropic"
            );
            assert_eq!(
                sniff(&no_hdrs(), p),
                WireProtocol::OpenAiChat,
                "path {p} without header → default OpenAiChat"
            );
        }
    }

    #[test]
    fn non_v1_path_returns_none() {
        // Not under /v1 → None (proxy maps to 404)
        assert_eq!(resolve_target(&no_hdrs(), "/healthz"), None);
        assert_eq!(resolve_target(&no_hdrs(), "/foo/bar"), None);
        assert_eq!(resolve_target(&no_hdrs(), "/v1foo"), None); // segment boundary
        assert_eq!(resolve_target(&no_hdrs(), "/v1models"), None);

        // Explicit prefix but bare path not under /v1 → still None
        assert_eq!(resolve_target(&no_hdrs(), "/anthropic/foo"), None);
        assert_eq!(resolve_target(&no_hdrs(), "/openai/healthz"), None);

        // /v1 root itself is valid
        let t = resolve_target(&no_hdrs(), "/v1").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAiChat); // default
        assert_eq!(t.path, "/v1");
    }

    #[test]
    fn segment_boundary_no_false_match() {
        // /v1/messagesfoo must NOT match the /v1/messages entry → default OpenAiChat,
        // proving the Anthropic table did not match.
        assert_eq!(sniff(&no_hdrs(), "/v1/messagesfoo"), WireProtocol::OpenAiChat);
        assert_ne!(
            sniff(&no_hdrs(), "/v1/messagesfoo"),
            WireProtocol::Anthropic
        );

        // /v1/chat/completionsXYZ with anthropic-version header → Anthropic,
        // proving the OpenAI chat table did NOT match (table wins over header, so a
        // false match would yield OpenAiChat instead).
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/chat/completionsXYZ"),
            WireProtocol::Anthropic
        );
        // sanity: the real /v1/chat/completions with header → still OpenAiChat (table wins)
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/chat/completions"),
            WireProtocol::OpenAiChat
        );
    }

    #[test]
    fn anthropic_version_header_arbitrates_unknown_v1_path() {
        // A /v1 path not in any table, no explicit prefix → header decides
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/whoknows"),
            WireProtocol::Anthropic
        );
        assert_eq!(sniff(&no_hdrs(), "/v1/whoknows"), WireProtocol::OpenAiChat);

        let t = resolve_target(&hdrs_with(Some("2023-06-01")), "/v1/whoknows").unwrap();
        assert_eq!(t.protocol, WireProtocol::Anthropic);
        assert_eq!(t.path, "/v1/whoknows");
    }

    #[test]
    fn serde_roundtrip_uses_explicit_names() {
        assert_eq!(
            serde_json::from_str::<WireProtocol>("\"anthropic\"").unwrap(),
            WireProtocol::Anthropic
        );
        assert_eq!(
            serde_json::from_str::<WireProtocol>("\"openai_chat\"").unwrap(),
            WireProtocol::OpenAiChat
        );
        assert_eq!(
            serde_json::from_str::<WireProtocol>("\"openai_responses\"").unwrap(),
            WireProtocol::OpenAiResponses
        );
        assert_eq!(
            serde_json::to_string(&WireProtocol::OpenAiResponses).unwrap(),
            "\"openai_responses\""
        );
    }
}
