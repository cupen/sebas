use serde::{Deserialize, Serialize};

/// 上游 provider 的 API 协议面。纯透传模式下决定请求/响应的格式归约
/// （Anthropic 客户端走 Anthropic provider，OpenAI 同理），不做协议转换。
///
/// Renamed from `Protocol` to disambiguate from
/// `acp_claude::AgentProtocol` (which carries the same meaning but at the
/// agent→upstream seam, not the gateway→upstream seam).
///
/// serde `rename_all = "lowercase"`：`Anthropic` <-> `"anthropic"`，
/// `OpenAi` <-> `"openai"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireProtocol {
    Anthropic,
    OpenAi,
}

impl WireProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            WireProtocol::Anthropic => "anthropic",
            WireProtocol::OpenAi => "openai",
        }
    }
}

/// 协议嗅探结果：解析出的协议 + 剥离显式前缀后的 bare `/v1/...` 路径。
/// `path` 恒以 `/v1` 开头（段边界），由 `resolve_target` 保证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub protocol: WireProtocol,
    pub path: String,
}

/// Anthropic 专属路径表（见 openspec/specs/gateway-core/spec.md）。段边界感知匹配：
/// `/v1/messages` 命中 `/v1/messages` 与 `/v1/messages/x`，不命中 `/v1/messagesXYZ`。
const ANTHROPIC_PATHS: &[&str] = &["/v1/messages"];

/// OpenAI 专属路径表（见 openspec/specs/gateway-core/spec.md）。碰撞路径（`/v1/models`、`/v1/files`、
/// `/v1/skills`）刻意不入表，由 `anthropic-version` header 仲裁。
///
/// ⚠️ **仅外部 OpenAI 客户端使用** —— sebas 自身走 Gateway 模式时，
/// agent 只发 Anthropic 协议，本表对 sebas→gateway→upstream 路径不可见。
/// 见 openspec/specs/provider-management/spec.md。
const OPENAI_PATHS: &[&str] = &[
    "/v1/chat/completions",
    "/v1/responses",
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
fn explicit_prefix(path: &str) -> Option<(WireProtocol, &str)> {
    if let Some(rest) = path.strip_prefix("/anthropic")
        && (rest.starts_with('/') || rest.is_empty())
    {
        return Some((WireProtocol::Anthropic, rest));
    }
    if let Some(rest) = path.strip_prefix("/openai")
        && (rest.starts_with('/') || rest.is_empty())
    {
        return Some((WireProtocol::OpenAi, rest));
    }
    None
}

/// 判断路径是否落在 `/v1` 命名空间（段边界：`/v1` 或 `/v1/...`，非 `/v1foo`）。
fn is_under_v1(path: &str) -> bool {
    path == "/v1" || path.starts_with("/v1/")
}

/// 协议嗅探（见 openspec/specs/gateway-core/spec.md）。优先级（高 → 低）：
/// 1. 显式前缀 `/anthropic/`、`/openai/`（强制协议）
/// 2. Anthropic 专属路径表（`/v1/messages`）
/// 3. OpenAI 专属路径表
/// 4. `anthropic-version` header
/// 5. 默认 OpenAi
///
/// `path` 可带显式前缀（裸 `uri_path`）或 bare `/v1/...`，两种都能识别。
pub fn sniff(headers: &axum::http::HeaderMap, path: &str) -> WireProtocol {
    if let Some((proto, _)) = explicit_prefix(path) {
        return proto;
    }
    if ANTHROPIC_PATHS.iter().any(|e| path_matches_entry(path, e)) {
        return WireProtocol::Anthropic;
    }
    if OPENAI_PATHS.iter().any(|e| path_matches_entry(path, e)) {
        return WireProtocol::OpenAi;
    }
    if headers.contains_key("anthropic-version") {
        return WireProtocol::Anthropic;
    }
    WireProtocol::OpenAi
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

        // /openai/ forces OpenAi even on the Anthropic-specific /v1/messages
        let t = resolve_target(&no_hdrs(), "/openai/v1/messages").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAi);
        assert_eq!(t.path, "/v1/messages");

        // sniff agrees with the forced protocol
        assert_eq!(
            sniff(&no_hdrs(), "/anthropic/v1/chat/completions"),
            WireProtocol::Anthropic
        );
        assert_eq!(
            sniff(&no_hdrs(), "/openai/v1/messages"),
            WireProtocol::OpenAi
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
    fn openai_specific_paths_detected() {
        // Representative slice of the OpenAI path table; no header, no prefix
        for p in [
            "/v1/chat/completions",
            "/v1/responses",
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
                WireProtocol::OpenAi,
                "path {p} should be OpenAi"
            );
        }

        let t = resolve_target(&no_hdrs(), "/v1/chat/completions").unwrap();
        assert_eq!(t.protocol, WireProtocol::OpenAi);
        assert_eq!(t.path, "/v1/chat/completions");
    }

    #[test]
    fn collision_path_arbitrated_by_header_both_directions() {
        // /v1/models, /v1/files, /v1/skills are collision paths — not in any table.
        // anthropic-version header → Anthropic; absent → default OpenAi.
        for p in ["/v1/models", "/v1/files", "/v1/skills"] {
            assert_eq!(
                sniff(&hdrs_with(Some("2023-06-01")), p),
                WireProtocol::Anthropic,
                "path {p} + anthropic-version → Anthropic"
            );
            assert_eq!(
                sniff(&no_hdrs(), p),
                WireProtocol::OpenAi,
                "path {p} without header → default OpenAi"
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
        assert_eq!(t.protocol, WireProtocol::OpenAi); // default
        assert_eq!(t.path, "/v1");
    }

    #[test]
    fn segment_boundary_no_false_match() {
        // /v1/messagesfoo must NOT match the /v1/messages entry → default OpenAi,
        // proving the Anthropic table did not match.
        assert_eq!(sniff(&no_hdrs(), "/v1/messagesfoo"), WireProtocol::OpenAi);
        assert_ne!(
            sniff(&no_hdrs(), "/v1/messagesfoo"),
            WireProtocol::Anthropic
        );

        // /v1/chat/completionsXYZ with anthropic-version header → Anthropic,
        // proving the OpenAI table did NOT match (table wins over header, so a
        // false match would yield OpenAi instead).
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/chat/completionsXYZ"),
            WireProtocol::Anthropic
        );
        // sanity: the real /v1/chat/completions with header → still OpenAi (table wins)
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/chat/completions"),
            WireProtocol::OpenAi
        );
    }

    #[test]
    fn anthropic_version_header_arbitrates_unknown_v1_path() {
        // A /v1 path not in any table, no explicit prefix → header decides
        assert_eq!(
            sniff(&hdrs_with(Some("2023-06-01")), "/v1/whoknows"),
            WireProtocol::Anthropic
        );
        assert_eq!(sniff(&no_hdrs(), "/v1/whoknows"), WireProtocol::OpenAi);

        let t = resolve_target(&hdrs_with(Some("2023-06-01")), "/v1/whoknows").unwrap();
        assert_eq!(t.protocol, WireProtocol::Anthropic);
        assert_eq!(t.path, "/v1/whoknows");
    }
}
