//! 路由表与 model 提取（Task 4，spec §4.2）。
//!
//! - `RouteTable::from_config` / `RouteTable::resolve`：按优先级链解析 model
//!   → provider + 经 `model_map` 重命名后的 upstream_model。
//! - `glob_match` / `split_namespace` / `extract_model_from_body` /
//!   `extract_model_from_path`：路由层公共辅助。
//!
//! 优先级（高 → 低）：`provider/model` 命名空间（provider 须存在，否则按普通
//! model 名继续走）> 精确 > glob（按 `routes` 配置序取首命中）> key 级默认 >
//! 全局默认。model 缺失（GET 类）直接走默认链，`upstream_model` 为 `None`。
//!
//! 协议一致性：解析到的 `provider.protocol` ≠ 请求 `proto` → `ProtocolMismatch`，
//! 纯透传，不做协议转换。

use std::collections::HashMap;

use thiserror::Error;

use crate::config::{GatewayConfig, KeyConfig, ProviderConfig, RouteRule};
use crate::proto::Protocol;

/// 路由解析错误（spec §4.2）。proxy 按变体映射 HTTP 状态：
/// `NoRoute` → 502、`ProtocolMismatch` → 400、`ModelNotAllowed` → 403。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouteError {
    /// 无任何路由/默认可解析（proxy 映 502）。
    #[error("no route for model (no namespace/exact/glob/default matched)")]
    NoRoute,
    /// 解析到的 provider 协议与请求协议不一致（proxy 映 400）。
    #[error("protocol mismatch: provider '{provider}' does not speak the request protocol")]
    ProtocolMismatch { provider: String },
    /// key 的 `allow_models` 门禁拒绝该 model（proxy 映 403）。
    #[error("model not allowed by key allow_models")]
    ModelNotAllowed,
}

/// 路由决策：上游 provider 名 + 经 `model_map` 重命名后的 upstream_model。
/// `upstream_model` 为 `None` 仅当请求未携带 model（GET 类端点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub provider: String,
    pub upstream_model: Option<String>,
}

/// 路由表：从 `GatewayConfig` 一次性构建的只读结构，承担 model → provider 解析。
#[derive(Debug, Clone)]
pub struct RouteTable {
    providers: HashMap<String, ProviderConfig>,
    routes: Vec<RouteRule>,
    default_provider: Option<String>,
}

impl RouteTable {
    pub fn from_config(cfg: &GatewayConfig) -> RouteTable {
        // 唯一 provider 隐式默认：`default_provider` 未配置且 providers 恰有一个
        // 时，默认指向它——单 provider 场景可整体省略 `default_provider` 与 `routes`。
        let default_provider = cfg.default_provider.clone().or_else(|| {
            if cfg.providers.len() == 1 {
                cfg.providers.keys().next().cloned()
            } else {
                None
            }
        });
        RouteTable {
            providers: cfg.providers.clone(),
            routes: cfg.routes.clone(),
            default_provider,
        }
    }

    /// 按优先级链解析 model → provider + upstream_model。
    ///
    /// 优先级：命名空间 > 精确 > glob > key 默认 > 全局默认。
    /// `model` 为 `None`（GET 类）直接走默认链，`upstream_model` 置 `None`。
    pub fn resolve(
        &self,
        model: Option<&str>,
        proto: Protocol,
        key: Option<&KeyConfig>,
    ) -> Result<RouteDecision, RouteError> {
        // allow_models 门禁：key 有限制且 model 已知时，无 glob 命中 → 拒绝。
        // 门禁在路由前做，按客户端原始 model 串判定（spec §4.5）。
        if let Some(k) = key
            && !k.allow_models.is_empty()
            && let Some(m) = model
        {
            let allowed = k.allow_models.iter().any(|p| glob_match(p, m));
            if !allowed {
                return Err(RouteError::ModelNotAllowed);
            }
        }

        // 解析 (provider 名, 待 rename 的 model)。model_for_map 为 None 仅当
        // 请求未携带 model（GET 类）；命名空间命中时取 `rest`，其余取原 model。
        let (provider_name, model_for_map): (String, Option<&str>) = match model {
            None => {
                let p = self.default_provider_for(key).ok_or(RouteError::NoRoute)?;
                (p, None)
            }
            Some(m) => {
                // 命名空间：provider/model（provider 须存在，否则按普通 model 名走）
                if let Some((ns, rest)) = split_namespace(m)
                    && self.providers.contains_key(ns)
                {
                    (ns.to_string(), Some(rest))
                } else {
                    let p: String = match self.match_route(m) {
                        Some(p) => p.to_string(),
                        None => self.default_provider_for(key).ok_or(RouteError::NoRoute)?,
                    };
                    (p, Some(m))
                }
            }
        };

        // 协议一致性：纯透传，不做协议转换。
        // provider 存在性由 from_config 镜像 + config.rs validate 保证。
        let provider_cfg = self
            .providers
            .get(&provider_name)
            .expect("provider existence guaranteed by from_config mirror + config::validate");
        if provider_cfg.protocol != proto {
            return Err(RouteError::ProtocolMismatch {
                provider: provider_name,
            });
        }

        // model_map 重命名：命中取改名值，未命中原样透传。无 model → None。
        let upstream_model = model_for_map.map(|m| {
            provider_cfg
                .model_map
                .get(m)
                .cloned()
                .unwrap_or_else(|| m.to_string())
        });

        Ok(RouteDecision {
            provider: provider_name,
            upstream_model,
        })
    }

    /// 按 `routes` 配置序匹配 model。精确优先于 glob：先扫一遍精确相等，
    /// 再扫一遍 glob 命中；各自取首个命中。无命中 → `None`。
    fn match_route(&self, model: &str) -> Option<&str> {
        for r in &self.routes {
            if r.model == model {
                return Some(&r.provider);
            }
        }
        for r in &self.routes {
            if r.model.contains('*') && glob_match(&r.model, model) {
                return Some(&r.provider);
            }
        }
        None
    }

    /// 默认链：key 级 `default_provider` 优先于全局 `default_provider`。
    fn default_provider_for(&self, key: Option<&KeyConfig>) -> Option<String> {
        if let Some(k) = key
            && let Some(dp) = &k.default_provider
        {
            return Some(dp.clone());
        }
        self.default_provider.clone()
    }
}

/// 手写 glob 匹配（无 glob crate）。`*` 匹配任意字符（含空）；无 `*` 时精确相等。
/// 首段必须前缀、末段必须后缀（pattern 以 `*` 结尾时末段为空，`ends_with("")`
/// 恒真即天然豁免）、中段按序子串查找（各段须在前一段匹配尾之后出现）。
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().expect("split 至少产一个段");
    let second = match parts.next() {
        Some(seg) => seg,
        None => return pattern == s, // 无 '*'：精确相等
    };
    // 有至少一个 '*'。首段必须前缀。
    if !s.starts_with(first) {
        return false;
    }
    let mut pos = first.len();
    // 从 `second` 起逐段：每读出下一段时，当前段为中段（须按序子串命中）；
    // 迭代结束时，当前段为末段（须为后缀；pattern 以 '*' 结尾则末段为空，
    // `ends_with("")` 恒真，天然豁免后缀要求）。
    let mut seg = second;
    for next in parts {
        match s[pos..].find(seg) {
            Some(idx) => pos += idx + seg.len(),
            None => return false,
        }
        seg = next;
    }
    s[pos..].ends_with(seg)
}

/// 拆分 `provider/model` 命名空间。返回 `(provider, rest)`；无 `/` 返 `None`。
/// 不校验 provider 是否存在（由 `resolve` 在路由表上判定）。
pub fn split_namespace(model: &str) -> Option<(&str, &str)> {
    model.split_once('/')
}

/// 从 POST JSON body 提取 `model` 字段。缺字段 / 非 JSON / 非字符串 → `None`。
pub fn extract_model_from_body(body: &axum::body::Bytes) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body.as_ref()).ok()?;
    v.get("model")?.as_str().map(|s| s.to_string())
}

/// 从路径提取 model：仅 `/v1/models/{model}` 单层。嵌套或其它路径 → `None`。
pub fn extract_model_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/v1/models/")?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GatewayConfig, KeyConfig, ProviderConfig, RouteRule};
    use crate::proto::Protocol;
    use axum::body::Bytes;
    use std::collections::HashMap;

    /// 构造一个最小可用的 `GatewayConfig`（直接 struct，不经 parse/env，避免
    /// 与 config.rs 的 env-lock 测试串行约束耦合）。
    fn build_cfg(
        providers: HashMap<String, ProviderConfig>,
        routes: &[(&str, &str)],
        default: Option<&str>,
        keys: Vec<KeyConfig>,
    ) -> GatewayConfig {
        GatewayConfig {
            listen: "127.0.0.1:8787".into(),
            max_body_bytes: 67_108_864,
            connect_timeout_secs: 10,
            read_timeout_secs: 600,
            usage_file: "/tmp/sebas-gateway-usage.jsonl".into(),
            default_provider: default.map(String::from),
            keys,
            providers,
            routes: routes
                .iter()
                .map(|(m, p)| RouteRule {
                    model: (*m).to_string(),
                    provider: (*p).to_string(),
                })
                .collect(),
        }
    }

    fn simple_provider(name: &str, proto: Protocol) -> (String, ProviderConfig) {
        (
            name.to_string(),
            ProviderConfig {
                protocol: proto,
                base_url: format!("https://{name}.example.com"),
                api_key_env: None,
                api_key: Some("test-key".into()),
                model_map: HashMap::new(),
            },
        )
    }

    fn simple_providers(names: &[(&str, Protocol)]) -> HashMap<String, ProviderConfig> {
        names.iter().map(|(n, p)| simple_provider(n, *p)).collect()
    }

    fn key_with(allow: &[&str], default: Option<&str>) -> KeyConfig {
        KeyConfig {
            key: "sk-test".into(),
            key_env: None,
            name: "test".into(),
            rpm: None,
            daily_token_quota: None,
            allow_models: allow.iter().map(|s| (*s).to_string()).collect(),
            default_provider: default.map(String::from),
        }
    }

    // -------------------- glob_match 各形态 --------------------

    #[test]
    fn glob_no_star_is_exact() {
        assert!(glob_match("claude-sonnet", "claude-sonnet"));
        assert!(!glob_match("claude-sonnet", "claude-opus"));
        assert!(!glob_match("claude-sonnet", ""));
        assert!(glob_match("", "")); // 空 pattern 精确匹配空串
    }

    #[test]
    fn glob_prefix_star() {
        assert!(glob_match("claude-*", "claude-sonnet"));
        assert!(glob_match("claude-*", "claude-")); // * 可匹配空
        assert!(!glob_match("claude-*", "claude")); // 缺少 "-"
    }

    #[test]
    fn glob_suffix_star() {
        assert!(glob_match("*-sonnet", "claude-sonnet"));
        assert!(glob_match("*-sonnet", "x-sonnet"));
        assert!(!glob_match("*-sonnet", "claude-opus"));
    }

    #[test]
    fn glob_middle_star() {
        assert!(glob_match("claude-*-4", "claude-sonnet-4"));
        assert!(glob_match("claude-*-4", "claude--4")); // 中段匹配空
        assert!(!glob_match("claude-*-4", "claude-sonnet-3"));
        assert!(!glob_match("claude-*-4", "claude-4")); // 无 "-4" 后缀
    }

    #[test]
    fn glob_no_match() {
        assert!(!glob_match("claude-*", "gpt-4"));
        assert!(!glob_match("gpt-*", "claude-sonnet"));
    }

    #[test]
    fn glob_star_only_matches_anything() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "claude-sonnet-4"));
    }

    // -------------------- 命名空间 --------------------

    #[test]
    fn namespace_direct_routes_to_named_provider() {
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[],
            Some("anthropic"),
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        // "anthropic/claude-sonnet" → provider anthropic，model claude-sonnet
        let d = table
            .resolve(Some("anthropic/claude-sonnet"), Protocol::Anthropic, None)
            .expect("namespace direct should resolve");
        assert_eq!(d.provider, "anthropic");
        assert_eq!(d.upstream_model.as_deref(), Some("claude-sonnet"));
    }

    #[test]
    fn unknown_namespace_falls_back_to_normal_model() {
        // "foo/claude-sonnet"：foo 不是 provider → 整串作为普通 model 走精确/glob。
        // 配置精确路由 "foo/claude-sonnet" → openai 命中。
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[("foo/claude-sonnet", "openai")],
            Some("anthropic"),
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("foo/claude-sonnet"), Protocol::OpenAi, None)
            .expect("unknown namespace should fall back to exact route");
        assert_eq!(d.provider, "openai");
        assert_eq!(d.upstream_model.as_deref(), Some("foo/claude-sonnet"));
    }

    // -------------------- 优先级 --------------------

    #[test]
    fn exact_beats_glob_despite_order() {
        // route[0]: claude-* → anthropic（glob）
        // route[1]: claude-sonnet → openai（精确）
        // 即便 glob 在前，精确优先 → openai。
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[("claude-*", "anthropic"), ("claude-sonnet", "openai")],
            None,
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi, None)
            .expect("exact should match");
        assert_eq!(d.provider, "openai");
    }

    #[test]
    fn key_default_beats_global_default() {
        // 全局默认 anthropic；key 级默认 openai。model "gpt-4" 无路由 → 默认链
        // → key 级优先 → openai。
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[],
            Some("anthropic"),
            vec![key_with(&[], Some("openai"))],
        );
        let table = RouteTable::from_config(&cfg);
        let key = cfg.keys.first().unwrap();
        let d = table
            .resolve(Some("gpt-4"), Protocol::OpenAi, Some(key))
            .expect("key default should resolve");
        assert_eq!(d.provider, "openai");
        assert_eq!(d.upstream_model.as_deref(), Some("gpt-4"));
    }

    // -------------------- 协议一致性 --------------------

    #[test]
    fn protocol_mismatch_returns_error() {
        // route claude-* → anthropic（Anthropic）。请求协议 OpenAi → 不一致。
        let cfg = build_cfg(
            simple_providers(&[("anthropic", Protocol::Anthropic)]),
            &[("claude-*", "anthropic")],
            None,
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        let err = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi, None)
            .expect_err("protocol mismatch");
        assert_eq!(
            err,
            RouteError::ProtocolMismatch {
                provider: "anthropic".into()
            }
        );
    }

    // -------------------- 默认缺失 --------------------

    #[test]
    fn no_default_and_no_route_yields_no_route() {
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[],
            None,
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        // 两个 provider 且无默认/路由 → 无隐式默认，NoRoute。
        let err = table
            .resolve(Some("gpt-4"), Protocol::OpenAi, None)
            .expect_err("no route should error");
        assert_eq!(err, RouteError::NoRoute);
    }

    // -------------------- 唯一 provider 隐式默认 --------------------

    #[test]
    fn single_provider_becomes_implicit_default() {
        // 唯一 provider，无 default_provider、无 routes：
        // model 请求与无 model（GET 类）请求都应落到该 provider。
        let cfg = build_cfg(
            simple_providers(&[("anthropic", Protocol::Anthropic)]),
            &[],
            None,
            vec![],
        );
        let table = RouteTable::from_config(&cfg);

        let d = table
            .resolve(Some("claude-sonnet"), Protocol::Anthropic, None)
            .expect("single provider should implicitly default for model requests");
        assert_eq!(d.provider, "anthropic");
        assert_eq!(d.upstream_model.as_deref(), Some("claude-sonnet"));

        let d = table
            .resolve(None, Protocol::Anthropic, None)
            .expect("single provider should implicitly default for model-less requests");
        assert_eq!(d.provider, "anthropic");
        assert_eq!(d.upstream_model, None);
    }

    #[test]
    fn single_provider_implicit_default_does_not_hide_protocol_mismatch() {
        // 隐式默认仍受协议一致性约束：唯一 anthropic provider 收到 OpenAI
        // 协议请求 → ProtocolMismatch（而非静默转发）。
        let cfg = build_cfg(
            simple_providers(&[("anthropic", Protocol::Anthropic)]),
            &[],
            None,
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        let err = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi, None)
            .expect_err("protocol mismatch must still surface");
        assert_eq!(
            err,
            RouteError::ProtocolMismatch {
                provider: "anthropic".into()
            }
        );
    }

    // -------------------- allow_models 门禁 --------------------

    #[test]
    fn allow_models_pass_and_deny() {
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[("claude-*", "anthropic"), ("gpt-*", "openai")],
            Some("anthropic"),
            vec![key_with(&["claude-*"], None)],
        );
        let table = RouteTable::from_config(&cfg);
        let key = cfg.keys.first().unwrap();
        // 放行：claude-sonnet 命中 allow_models glob
        let d = table
            .resolve(Some("claude-sonnet"), Protocol::Anthropic, Some(key))
            .expect("allow_models should pass claude-*");
        assert_eq!(d.provider, "anthropic");
        // 拒绝：gpt-4 不在 allow_models
        let err = table
            .resolve(Some("gpt-4"), Protocol::OpenAi, Some(key))
            .expect_err("allow_models should deny gpt-4");
        assert_eq!(err, RouteError::ModelNotAllowed);
    }

    // -------------------- model_map 重命名 --------------------

    #[test]
    fn model_map_rename_and_passthrough() {
        // provider bedrock 的 model_map：claude-sonnet → anthropic.claude-sonnet-4
        // 未命中的 model 原样透传。
        let mut providers = simple_providers(&[("bedrock", Protocol::Anthropic)]);
        if let Some(b) = providers.get_mut("bedrock") {
            b.model_map
                .insert("claude-sonnet".into(), "anthropic.claude-sonnet-4".into());
        }
        let cfg = build_cfg(providers, &[], Some("bedrock"), vec![]);
        let table = RouteTable::from_config(&cfg);
        // 命中 model_map → 改名
        let d = table
            .resolve(Some("claude-sonnet"), Protocol::Anthropic, None)
            .expect("mapped model resolves");
        assert_eq!(d.provider, "bedrock");
        assert_eq!(
            d.upstream_model.as_deref(),
            Some("anthropic.claude-sonnet-4")
        );
        // 未命中 → 原样
        let d2 = table
            .resolve(Some("claude-opus"), Protocol::Anthropic, None)
            .expect("unmapped model resolves");
        assert_eq!(d2.provider, "bedrock");
        assert_eq!(d2.upstream_model.as_deref(), Some("claude-opus"));
    }

    // -------------------- 无 model（GET 类）--------------------

    #[test]
    fn no_model_uses_default_and_upstream_none() {
        let cfg = build_cfg(
            simple_providers(&[
                ("anthropic", Protocol::Anthropic),
                ("openai", Protocol::OpenAi),
            ]),
            &[],
            Some("openai"),
            vec![],
        );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(None, Protocol::OpenAi, None)
            .expect("default should resolve when model absent");
        assert_eq!(d.provider, "openai");
        assert_eq!(d.upstream_model, None);
    }

    // -------------------- body 提取 --------------------

    #[test]
    fn extract_model_from_body_cases() {
        // 正常
        let b = Bytes::from(r#"{"model":"claude-sonnet"}"#);
        assert_eq!(
            extract_model_from_body(&b).as_deref(),
            Some("claude-sonnet")
        );
        // 缺字段
        assert_eq!(
            extract_model_from_body(&Bytes::from(r#"{"foo":"bar"}"#)),
            None
        );
        // 空 body
        assert_eq!(extract_model_from_body(&Bytes::from("")), None);
        // 非 JSON
        assert_eq!(extract_model_from_body(&Bytes::from("not json")), None);
        // 非字符串（数字）
        assert_eq!(
            extract_model_from_body(&Bytes::from(r#"{"model":42}"#)),
            None
        );
        // 非字符串（对象）
        assert_eq!(
            extract_model_from_body(&Bytes::from(r#"{"model":{"x":1}}"#)),
            None
        );
    }

    // -------------------- path 提取 --------------------

    #[test]
    fn extract_model_from_path_cases() {
        // 正常单层
        assert_eq!(
            extract_model_from_path("/v1/models/claude-sonnet").as_deref(),
            Some("claude-sonnet")
        );
        // 嵌套拒绝
        assert_eq!(extract_model_from_path("/v1/models/foo/bar"), None);
        // 无 id
        assert_eq!(extract_model_from_path("/v1/models"), None);
        assert_eq!(extract_model_from_path("/v1/models/"), None);
        // 其它路径
        assert_eq!(extract_model_from_path("/v1/chat/completions"), None);
        assert_eq!(extract_model_from_path("/healthz"), None);
    }
}
