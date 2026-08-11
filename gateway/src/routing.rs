//! 路由表与 model 提取（Task 4，spec §4.2）。
//!
//! - `RouteTable::from_config` / `RouteTable::resolve`：按优先级链解析 model
//!   → provider + 经 `model_map` 重命名后的 upstream_model。
//! - `glob_match` / `split_namespace` / `extract_model_from_body` /
//!   `extract_model_from_path`：路由层公共辅助。
//!
//! 优先级（高 → 低）：`provider/model` 命名空间（provider 须存在，否则按普通
//! model 名继续走）> 精确 > glob（routes 按 model 名排序，glob 撞车取字典序
//! 首个命中；每个路由组内 provider 数组顺序即优先级，当前取第一个）> 全局默认
//! （唯一 provider 的隐式默认折叠进 `default_provider`）。model 缺失（GET 类）
//! 直接走默认链，`upstream_model` 为 `None`。
//!
//! 协议一致性：解析到的 `provider.protocol` ≠ 请求 `proto` → `ProtocolMismatch`，
//! 纯透传，不做协议转换。

use std::collections::HashMap;

use thiserror::Error;

use crate::config::{GatewayConfig, ProviderConfig, RouteGroup};
use crate::proto::Protocol;

/// 路由解析错误（spec §4.2）。proxy 按变体映射 HTTP 状态：
/// `NoRoute` → 502、`ProtocolMismatch` → 400。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouteError {
    /// 无任何路由/默认可解析（proxy 映 502）。
    #[error("no route for model (no namespace/exact/glob/default matched)")]
    NoRoute,
    /// 解析到的 provider 协议与请求协议不一致（proxy 映 400）。
    #[error("protocol mismatch: provider '{provider}' does not speak the request protocol")]
    ProtocolMismatch { provider: String },
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
    routes: Vec<RouteGroup>,
    default_provider: Option<String>,
    /// debug 模式：内置 `test` provider 由 gateway 自身应答，绕过
    /// 协议一致性检查（双协议面都可命中）。
    debug: bool,
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
            debug: cfg.debug,
        }
    }

    /// 按优先级链解析 model → provider + upstream_model。
    ///
    /// 优先级：命名空间 > 精确 > glob > 全局默认（唯一 provider 的隐式默认
    /// 已在 `from_config` 折叠进 `default_provider`）。
    /// `model` 为 `None`（GET 类）直接走默认链，`upstream_model` 置 `None`。
    pub fn resolve(
        &self,
        model: Option<&str>,
        proto: Protocol,
    ) -> Result<RouteDecision, RouteError> {
        // 解析 (provider 名, 待 rename 的 model)。model_for_map 为 None 仅当
        // 请求未携带 model（GET 类）；命名空间命中时取 `rest`，其余取原 model。
        let (provider_name, model_for_map): (String, Option<&str>) = match model {
            None => {
                let p = self.default_provider.clone().ok_or(RouteError::NoRoute)?;
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
                        None => self
                            .default_provider
                            .clone()
                            .ok_or(RouteError::NoRoute)?,
                    };
                    (p, Some(m))
                }
            }
        };

        // 协议一致性：纯透传，不做协议转换。
        // provider 存在性由 from_config 镜像 + config.rs validate 保证；
        // 现在每个 provider 按请求协议选 URL：缺该协议位 → ProtocolMismatch。
        let provider_cfg = self
            .providers
            .get(&provider_name)
            .expect("provider existence guaranteed by from_config mirror + config::validate");
        if provider_cfg.url_for(proto).is_none() && !(self.debug && provider_name == "test") {
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

    /// 按 routes 匹配 model：先精确、后 glob（routes 已按 model 名排序，
    /// glob 撞车时字典序首个命中）。每个路由组取 provider 数组第一个（主）。
    /// 无命中 → `None`。
    fn match_route(&self, model: &str) -> Option<&str> {
        for r in &self.routes {
            if r.model == model {
                // TODO(故障转移): provider 数组已按优先级排好（先 = 主），
                // 当前只取第一个；实现时改为「主失败（网络/5xx）按序切换
                // 下一个」，并注意 SSE 只能在响应头发出前切换、超时预算、
                // usage 结算语义。
                return r.providers.first().map(String::as_str);
            }
        }
        for r in &self.routes {
            if r.model.contains('*') && glob_match(&r.model, model) {
                // TODO(故障转移): 同上一处，glob 命中后同样只取主 provider。
                return r.providers.first().map(String::as_str);
            }
        }
        None
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
    use crate::config::{GatewayConfig, ProviderConfig, RouteGroup};
    use crate::proto::Protocol;
    use axum::body::Bytes;
    use std::collections::HashMap;

    /// 构造一个最小可用的 `GatewayConfig`（直接 struct，不经 parse/env，避免
    /// 与 config.rs 的 env-lock 测试串行约束耦合）。
    fn build_cfg(
        providers: HashMap<String, ProviderConfig>,
        routes: &[(&str, &[&str])],
        default: Option<&str>,
    ) -> GatewayConfig {
        GatewayConfig {
            listen: "127.0.0.1:8787".into(),
            max_body_bytes: 67_108_864,
            connect_timeout_secs: 10,
            read_timeout_secs: 600,
            usage_file: "/tmp/sebas-gateway-usage.jsonl".into(),
            debug: false,
            provider_overlay: "__test_no_overlay__.json".into(),
            default_provider: default.map(String::from),
            auth_token: Vec::new(),
            providers,
            routes: routes
                .iter()
                .map(|(m, ps)| RouteGroup {
                    model: (*m).to_string(),
                    providers: ps.iter().map(|p| (*p).to_string()).collect(),
                })
                .collect(),
        }
    }

    fn simple_provider(name: &str, proto: Protocol) -> (String, ProviderConfig) {
        let url = format!("https://{name}.example.com");
        (
            name.to_string(),
            ProviderConfig {
                base_url_anthropic: (proto == Protocol::Anthropic).then(|| url.clone()),
                base_url_openai: (proto == Protocol::OpenAi).then(|| url),
                api_key_env: None,
                api_key: Some("test-key".into()),
                model_map: HashMap::new(),
            },
        )
    }

    fn simple_providers(names: &[(&str, Protocol)]) -> HashMap<String, ProviderConfig> {
        names.iter().map(|(n, p)| simple_provider(n, *p)).collect()
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
            Some("anthropic")
                    );
        let table = RouteTable::from_config(&cfg);
        // "anthropic/claude-sonnet" → provider anthropic，model claude-sonnet
        let d = table
            .resolve(Some("anthropic/claude-sonnet"), Protocol::Anthropic)
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
            &[("foo/claude-sonnet", &["openai"])],
            Some("anthropic")
                    );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("foo/claude-sonnet"), Protocol::OpenAi)
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
            &[("claude-*", &["anthropic"]), ("claude-sonnet", &["openai"])],
            None
                    );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi)
            .expect("exact should match");
        assert_eq!(d.provider, "openai");
    }

    #[test]
    fn route_provider_array_priority_takes_first() {
        // 同一 model 的 provider 数组顺序 = 优先级：取第一个。
        let cfg = build_cfg(
            simple_providers(&[
                ("deepseek", Protocol::Anthropic),
                ("ark", Protocol::Anthropic),
            ]),
            &[("deepseek-chat", &["deepseek", "ark"])],
            None
                    );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(Some("deepseek-chat"), Protocol::Anthropic)
            .expect("deepseek-chat should resolve");
        assert_eq!(d.provider, "deepseek", "first provider in array must win");
    }

    // -------------------- 协议一致性 --------------------

    #[test]
    fn protocol_mismatch_returns_error() {
        // route claude-* → anthropic（Anthropic）。请求协议 OpenAi → 不一致。
        let cfg = build_cfg(
            simple_providers(&[("anthropic", Protocol::Anthropic)]),
            &[("claude-*", &["anthropic"])],
            None
                    );
        let table = RouteTable::from_config(&cfg);
        let err = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi)
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
            None
                    );
        let table = RouteTable::from_config(&cfg);
        // 两个 provider 且无默认/路由 → 无隐式默认，NoRoute。
        let err = table
            .resolve(Some("gpt-4"), Protocol::OpenAi)
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
            None
                    );
        let table = RouteTable::from_config(&cfg);

        let d = table
            .resolve(Some("claude-sonnet"), Protocol::Anthropic)
            .expect("single provider should implicitly default for model requests");
        assert_eq!(d.provider, "anthropic");
        assert_eq!(d.upstream_model.as_deref(), Some("claude-sonnet"));

        let d = table
            .resolve(None, Protocol::Anthropic)
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
            None
                    );
        let table = RouteTable::from_config(&cfg);
        let err = table
            .resolve(Some("claude-sonnet"), Protocol::OpenAi)
            .expect_err("protocol mismatch must still surface");
        assert_eq!(
            err,
            RouteError::ProtocolMismatch {
                provider: "anthropic".into()
            }
        );
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
        let cfg = build_cfg(providers, &[], Some("bedrock"));
        let table = RouteTable::from_config(&cfg);
        // 命中 model_map → 改名
        let d = table
            .resolve(Some("claude-sonnet"), Protocol::Anthropic)
            .expect("mapped model resolves");
        assert_eq!(d.provider, "bedrock");
        assert_eq!(
            d.upstream_model.as_deref(),
            Some("anthropic.claude-sonnet-4")
        );
        // 未命中 → 原样
        let d2 = table
            .resolve(Some("claude-opus"), Protocol::Anthropic)
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
            Some("openai")
                    );
        let table = RouteTable::from_config(&cfg);
        let d = table
            .resolve(None, Protocol::OpenAi)
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
